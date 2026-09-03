//! `hash-parity` — the gate that must pass before any sync code is trusted.
//!
//! Uploads crafted save files, reads back the `content_hash` the *server*
//! computed, and compares it to ours. Everything it creates is deleted again.
//!
//! Cases are chosen to hit the places a port realistically diverges: the zip
//! manifest path (where RomM itself shipped a bug), entry ordering, directory
//! entries, empty members, and non-ASCII names.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::api;
use crate::savehash;

#[derive(Debug, Deserialize)]
struct SaveResp {
    id: i64,
    content_hash: Option<String>,
}

struct Case {
    name: &'static str,
    file_name: String,
    path: PathBuf,
    what: &'static str,
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])], dirs: &[&str]) -> Result<()> {
    let f = std::fs::File::create(path)?;
    let mut w = zip::ZipWriter::new(f);
    let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for d in dirs {
        w.add_directory(*d, opts)?;
    }
    for (name, bytes) in entries {
        w.start_file(*name, opts)?;
        w.write_all(bytes)?;
    }
    w.finish()?;
    Ok(())
}

fn build_cases(dir: &Path) -> Result<Vec<Case>> {
    std::fs::create_dir_all(dir)?;
    let mut cases = Vec::new();

    // 1. Plain binary — the raw-MD5 path.
    let p = dir.join("plain.srm");
    std::fs::write(&p, (0u8..=255).cycle().take(9000).collect::<Vec<u8>>())?;
    cases.push(Case {
        name: "plain binary",
        file_name: "parity-plain.srm".into(),
        path: p,
        what: "raw MD5 of file bytes",
    });

    // 2. Empty file — boundary for the streaming loop.
    let p = dir.join("empty.srm");
    std::fs::write(&p, b"")?;
    cases.push(Case {
        name: "empty file",
        file_name: "parity-empty.srm".into(),
        path: p,
        what: "raw MD5 of nothing",
    });

    // 3. Zip written out of alphabetical order — proves sorted() is applied.
    let p = dir.join("unsorted.zip");
    write_zip(
        &p,
        &[("zz.bin", b"last"), ("aa.bin", b"first"), ("mm.bin", b"mid")],
        &[],
    )?;
    cases.push(Case {
        name: "zip, unsorted entries",
        file_name: "parity-unsorted.zip".into(),
        path: p,
        what: "manifest hash; entry order must be sorted",
    });

    // 4. Zip containing directory entries — those must be skipped.
    let p = dir.join("withdirs.zip");
    write_zip(
        &p,
        &[("sub/a.bin", b"aaa"), ("sub/b.bin", b"bbb")],
        &["sub/"],
    )?;
    cases.push(Case {
        name: "zip with directory entries",
        file_name: "parity-withdirs.zip".into(),
        path: p,
        what: "directory members skipped",
    });

    // 5. Zip with an empty member and a non-ASCII name.
    let p = dir.join("edge.zip");
    write_zip(&p, &[("empty.bin", b""), ("ünïcøde.bin", "héllo".as_bytes())], &[])?;
    cases.push(Case {
        name: "zip, empty + non-ASCII names",
        file_name: "parity-edge.zip".into(),
        path: p,
        what: "empty member, UTF-8 sort order",
    });

    Ok(cases)
}

async fn upload(
    client: &api::Client,
    rom_id: i64,
    case: &Case,
) -> Result<SaveResp> {
    let bytes = std::fs::read(&case.path)?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(case.file_name.clone())
        .mime_str("application/octet-stream")?;
    let form = reqwest::multipart::Form::new().part("saveFile", part);

    let url = format!(
        "{}/api/saves?rom_id={rom_id}&emulator=parity-test&slot=parity",
        client.base()
    );
    let resp = client
        .http()
        .post(&url)
        .header("Authorization", client.auth())
        .multipart(form)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("upload {} -> {status}\n  {}", case.file_name, body.chars().take(300).collect::<String>());
    }
    serde_json::from_str(&body)
        .with_context(|| format!("decoding save response: {}", body.chars().take(200).collect::<String>()))
}

async fn delete_saves(client: &api::Client, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let url = format!("{}/api/saves/delete", client.base());
    let resp = client
        .http()
        .post(&url)
        .header("Authorization", client.auth())
        .json(&serde_json::json!({ "saves": ids }))
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("cleanup failed: {} — remove save ids {ids:?} manually", resp.status());
    }
    Ok(())
}

pub async fn run(client: &api::Client) -> Result<()> {
    // Attach test saves to a real ROM; they are deleted at the end.
    let rom = client
        .roms(None, 1, 0, None)
        .await?
        .items
        .into_iter()
        .next()
        .context("server returned no ROMs")?;
    println!("using rom {} ({}) for test uploads\n", rom.id, rom.fs_name);

    let dir = std::env::temp_dir().join("moose-rack-parity");
    let cases = build_cases(&dir)?;

    let mut uploaded: Vec<i64> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        let ours = savehash::compute(&case.path)?;
        let result = upload(client, rom.id, case).await;
        match result {
            Ok(save) => {
                uploaded.push(save.id);
                match save.content_hash.as_deref() {
                    Some(theirs) if theirs.eq_ignore_ascii_case(&ours) => {
                        println!("  PASS  {:<32} {ours}", case.name);
                        println!("        {}", case.what);
                    }
                    Some(theirs) => {
                        println!("  FAIL  {:<32}", case.name);
                        println!("        ours   {ours}");
                        println!("        server {theirs}");
                        failures.push(case.name.to_owned());
                    }
                    None => {
                        println!("  ????  {:<32} server returned no content_hash", case.name);
                        failures.push(format!("{} (no server hash)", case.name));
                    }
                }
            }
            Err(e) => {
                println!("  ERR   {:<32} {e}", case.name);
                failures.push(format!("{} (upload failed)", case.name));
            }
        }
    }

    // Always clean up, even if assertions failed.
    let cleanup = delete_saves(client, &uploaded).await;
    std::fs::remove_dir_all(&dir).ok();
    println!("\ncleaned up {} test save(s)", uploaded.len());
    if let Err(e) = cleanup {
        println!("WARNING: {e}");
    }

    if failures.is_empty() {
        println!("\nhash parity PASSED across {} cases — safe to build sync on this.", cases.len());
        Ok(())
    } else {
        bail!(
            "hash parity FAILED for: {}\n\
             Do not build sync until this passes: a wrong hash means no_op never fires \
             and every save re-uploads forever.",
            failures.join(", ")
        )
    }
}
