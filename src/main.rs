//! Command-line entry point.
//!
//! The CLI and TUI here, and the Tauri GUI in `src-tauri/`, are three frontends
//! over the same `moose_rack` library. Run with no arguments for the command
//! list.
//!
//! Commands that touch the server (`check`, `sync`, `get`, `hash-parity`) need
//! `config.toml`; the rest work offline against the local cache.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use moose_rack::{
    cache, cores, download, media, parity, saves, theme, theme_remote, tui,
};
use moose_rack::config::Config;
use moose_rack::coremap::{self, CoreMap};
use moose_rack::retroarch::{self, RetroArch};
use moose_rack::retroarch_install;
use moose_rack::launch;
use moose_rack::probe;
use moose_rack::shaders;
use moose_rack::util::human;

/// Read from disk when it is there so the mapping can be edited without a
/// rebuild, falling back to the copy compiled into the binary.
///
/// The GUI has always done this. The CLI insisted on the file, so `doctor` and
/// `launch` failed outright on a Windows install that had only the executable
/// — the one platform where nobody has a source checkout to supply it.
const CORE_MAP: &str = "data/esde-core-map.json";

/// `[retroarch] root` from config.toml, if set.
/// Locate RetroArch using the configured boot order, with BIOS pointed at the
/// library's synced system folder.
fn locate_retroarch(cfg: &Config) -> Result<RetroArch> {
    Ok(RetroArch::locate_in(&cfg.retroarch.ordered_paths())?
        .with_system_dir(Some(cfg.system_dir())))
}

/// Infer the RomM platform slug from a ROM path under `.../roms/<slug>/...`.
///
/// Walks up to the directory whose parent is `roms`, so it works at any depth:
/// a plain ROM, a disc inside a `MultiDisk/` folder, or a file inside a
/// per-game folder ROM (`roms/dc/Shenmue (USA)/Shenmue (USA).m3u`).
fn platform_from_path(rom: &Path) -> Option<String> {
    let mut dir = rom.parent()?;
    loop {
        let parent = dir.parent()?;
        if parent.file_name().is_some_and(|n| n == "roms") {
            return Some(dir.file_name()?.to_str()?.to_owned());
        }
        dir = parent;
    }
}

/// Report what in config.toml no longer says what it used to, and with `--fix`,
/// change the ones that need no judgement.
///
/// A backup goes beside the file before anything is written. The config holds a
/// server token and, on an old enough file, a password: losing it to a bad edit
/// would cost more than the stale keys ever did.
fn cmd_config(fix: bool) -> Result<()> {
    use moose_rack::configpatch as cp;

    let path = std::path::Path::new("config.toml");
    if !path.is_file() {
        println!("no config.toml here — copy config.example.toml to make one");
        return Ok(());
    }
    let text = std::fs::read_to_string(path)?;
    let found = cp::inspect(&text);
    let stamped = cp::version_of(&text);

    if found.is_empty() {
        println!("config.toml is up to date (version {stamped} of {})", cp::CURRENT_VERSION);
        return Ok(());
    }

    println!("{} thing(s) worth knowing about config.toml:\n", found.len());
    for f in &found {
        println!("  [{}] {}", f.severity, f.what);
        for line in textwrap(&f.note, 74) {
            println!("      {line}");
        }
        println!("      {}", if f.fix.is_some() { "-> can be updated automatically" } else { "-> left alone: this one is your call" });
        println!();
    }

    let can = cp::fixable(&found).len();
    if !fix {
        if can > 0 {
            println!("{can} of these can be updated: moose-rack config --fix");
        }
        return Ok(());
    }
    if can == 0 {
        println!("nothing here can be updated without a decision from you");
        return Ok(());
    }

    let backup = path.with_extension("toml.before-patch");
    std::fs::copy(path, &backup)?;
    let (patched, applied) = cp::patch(&text);
    std::fs::write(path, &patched)?;
    println!("updated {} thing(s); the file as it was is in {}", applied.len(), backup.display());
    if applied.iter().any(|f| f.what.ends_with("password")) {
        println!("  note: that backup still has the password in it — delete it when you are happy");
    }
    for f in &applied {
        println!("  {}", f.what);
    }
    Ok(())
}

/// Wrap at a width, without pulling in a dependency for four lines of output.
fn textwrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn cmd_doctor() -> Result<()> {
    let cfg = Config::load()?;
    let ra = locate_retroarch(&cfg)?;
    println!("RetroArch");
    println!("  root      {}", ra.root.display());
    println!("  binary    {}", ra.binary.display());
    println!(
        "  portable  {}",
        if ra.portable {
            "yes (portable.txt present)"
        } else {
            "NO — saves/states go to ~/Documents/RetroArch"
        }
    );

    let installed = ra.installed_cores();
    println!(
        "  cores     {} installed at {}",
        installed.len(),
        ra.cores_dir().display()
    );

    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let mut ready = Vec::new();
    let mut missing = Vec::new();
    // Resolve the same way a launch does, so this reports what would actually
    // run rather than the ES-DE map's default — the two differ wherever
    // [cores.overrides] applies, which was silently misreported before.
    for platform in map.default_core_by_server_platform.keys() {
        let core = coremap::resolve_core(&map, &cfg.cores.overrides, platform, |c| ra.has_core(c))
            .or_else(|| map.default_core(platform).map(str::to_owned));
        let Some(core) = core else { continue };
        if ra.has_core(&core) {
            ready.push((platform.clone(), core));
        } else {
            missing.push((platform.clone(), core));
        }
    }

    println!(
        "\n{} of {} platforms ready to launch:",
        ready.len(),
        ready.len() + missing.len()
    );
    for (platform, core) in &ready {
        println!("  {platform:<16} {core}");
    }
    if !missing.is_empty() {
        println!("\n{} platforms need a core download:", missing.len());
        for (platform, core) in &missing {
            // Is a non-default alternative already installed?
            let alt = map
                .alternatives(platform)
                .into_iter()
                .find(|c| ra.has_core(c));
            match alt {
                Some(a) => println!("  {platform:<16} {core:<18} (alternative installed: {a})"),
                None => println!("  {platform:<16} {core}"),
            }
        }
    }
    Ok(())
}

async fn cmd_launch(
    rom: &Path,
    go: bool,
    core_override: Option<&str>,
    fullscreen: bool,
    pad: Option<&str>,
) -> Result<()> {
    let cfg = Config::load()?;
    let ra = RetroArch::locate(cfg.retroarch.root.as_deref())?
        .with_system_dir(Some(cfg.system_dir()));
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));

    // Ask the index first: it knows exactly where each game came from, which
    // path inference cannot for an ES-DE tree whose directories are ES-DE
    // system names rather than RomM slugs.
    let platform = cache::Cache::open(Path::new(CACHE_DB))
        .ok()
        .and_then(|c| c.platform_for_path(rom))
        .or_else(|| platform_from_path(rom))
        .with_context(|| format!("cannot infer platform from {}", rom.display()))?;
    let user_cfg = cfg.user_retroarch_config();
    let saves_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let achievements = cfg.achievements.settings();
    let req = launch::Request {
        saves_root: Some(&saves_root),
        // The same settings the window uses, so a dry run writes the same file
        // a real launch does.
        autofire: moose_rack::tweaks::AutoFire::parse(&cfg.retroarch.autofire),
        save_state_on_exit: cfg.retroarch.save_state_on_exit,
        autofire_hz: cfg.retroarch.autofire_hz,
        window_decorations: true,
        fit_window: true,
        mirror_players: true,
        entry_slot: None,
        rom,
        platform: &platform,
        fs_name: rom.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        library_root: Path::new(&cfg.library.local_root),
        user_cfg: &user_cfg,
        shaders_enabled: cfg.shaders.enabled,
        shader_overrides: &cfg.shaders.by_platform,
        core_overrides: &cfg.cores.overrides,
        core_per_game: &cfg.cores.per_game,
        lightgun: &cfg.lightgun.by_platform,
        // The CLI prints a command rather than owning a window, so it has no
        // display to measure and does not touch the emulator's window size.
        screen: None,
        core_override,
motion_shader: cfg.shaders.motion.as_deref(),
        refresh_hz: None,
        pad,
        achievements: Some(&achievements),
    };
    // Cores, shaders and BIOS, fetched on the way in rather than as advice to
    // go and run something. The GUI has always done this; the CLI told you to
    // install them yourself, which is the same information delivered at the
    // moment it is least useful.
    if let Ok(client) = cfg.server.client() {
        let core_wanted = core_override.map(str::to_owned).or_else(|| {
            coremap::resolve_core_for(
                &map, &cfg.cores.overrides, &cfg.cores.per_game,
                &platform, Some(req.fs_name), |_| true,
            )
        });
        if let Some(core) = core_wanted.as_deref() {
            match cores::ensure(client.http(), &ra, core).await {
                Ok(true) => println!("fetched the {core} core"),
                Ok(false) => {}
                Err(e) => eprintln!("warning: could not fetch {core}: {e}"),
            }
            match moose_rack::bios::ensure(
                &client, Path::new(&cfg.library.local_root), core, &platform,
            ).await {
                Ok(0) => {}
                Ok(n) => println!("fetched {n} BIOS file(s)"),
                Err(e) => eprintln!("warning: could not fetch BIOS: {e}"),
            }
        }
        if cfg.shaders.enabled
            && let Err(e) = shaders::ensure_pack(client.http(), &ra).await
        {
            eprintln!("warning: could not fetch shaders: {e}");
        }
    }

    let plan = launch::plan(&ra, &map, &req)?;

    for note in &plan.notes {
        println!("{note}");
    }
    if let Some(label) = &plan.shader_label {
        println!("shader  {label}");
    }
    let core_label = plan
        .core_label
        .as_ref()
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();
    println!("core    {}{core_label}", plan.core);
    // plan.rom, not the argument: a folder ROM launches the playlist inside it,
    // and printing the folder would name a path RetroArch never sees.
    println!("rom     {}", plan.rom.display());
    println!("command {}", retroarch::render(&plan.command(&ra, fullscreen)?));

    if !go {
        println!("\n(dry run — pass --go to actually launch)");
        return Ok(());
    }

    println!("\nlaunching…");
    let status = plan.run(&ra, fullscreen)?;
    match status.code() {
        Some(0) => println!("RetroArch exited cleanly (0)"),
        Some(c) => println!("RetroArch exited with status {c}"),
        None => println!("RetroArch terminated by signal"),
    }
    Ok(())
}

/// List one launchable ROM per platform that has an installed core.
fn cmd_suggest() -> Result<()> {
    let cfg = Config::load()?;
    let ra = locate_retroarch(&cfg)?;
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let roms = cfg.local_roms_dir();
    if !roms.is_dir() {
        bail!(
            "{} not found — build it with tools/build_test_library.py",
            roms.display()
        );
    }
    println!("launchable ROMs in {}:\n", roms.display());
    let mut shown = 0;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&roms)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let platform = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Some(core) = map.default_core(&platform) else {
            continue;
        };
        if !ra.has_core(core) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        if let Some(first) = files
            .flatten()
            .map(|f| f.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e != "m3u"))
            .min()
        {
            println!("  {platform:<16} {}", first.display());
            shown += 1;
        }
    }
    if shown == 0 {
        println!("  (none — no installed core matches any staged platform)");
    }
    Ok(())
}

/// Stage 1 — prove auth works and the library is reachable.
async fn cmd_check() -> Result<()> {
    let cfg = Config::load()?;
    let client = cfg.server.client()?;

    let me = client.me().await?;
    println!("server    {}", cfg.server.url);
    match client.heartbeat().await {
        Ok(hb) => {
            let v = &hb.system.version;
            let note = if v == moose_rack::VERIFIED_AGAINST {
                "verified".to_owned()
            } else {
                format!("UNVERIFIED — client was checked against {}", moose_rack::VERIFIED_AGAINST)
            };
            println!("version   server {v} ({note})");
        }
        Err(e) => println!("version   unknown ({e})"),
    }
    match client.config().await {
        Ok(sc) => println!(
            "hashing   {} excluded names, {} excluded exts{}",
            sc.default_excluded_files.len(),
            sc.default_excluded_extensions.len(),
            if sc.skip_hash_calculation { ", SKIP_HASH_CALCULATION set" } else { "" }
        ),
        Err(e) => println!("hashing   could not read /api/config ({e})"),
    }
    println!("user      {} (id {}, role {})", me.username, me.id, me.role);

    let count = client.rom_count().await?;
    println!("roms      {count}");

    let mut platforms = client.platforms().await?;
    platforms.retain(|p| p.rom_count > 0);
    platforms.sort_by_key(|p| std::cmp::Reverse(p.rom_count));
    println!("platforms {} populated", platforms.len());
    for p in platforms.iter().take(8) {
        println!("    {:<18} {}", p.fs_slug, p.rom_count);
    }
    if platforms.len() > 8 {
        println!("    … and {} more", platforms.len() - 8);
    }
    Ok(())
}

/// Show the shader assigned to each platform, and the alternatives.
fn cmd_shaders(platform: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    let ra = locate_retroarch(&cfg)?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;

    if let Some(slug) = platform {
        let display = shaders::display_of(slug);
        let current = shaders::preset_for(&cfg.shaders.by_platform, slug);
        println!("{slug} — {} display\n", match display {
            shaders::Display::Crt => "CRT / television",
            shaders::Display::Handheld => "handheld LCD",
        });
        for opt in shaders::available(&ra, display) {
            let mark = if current.as_deref() == Some(opt.path) { "*" } else { " " };
            println!("  {mark} {:<28} {:<34} {}", opt.label, opt.path, opt.note);
        }
        println!("  {} {:<28} no shader", if current.is_none() { "*" } else { " " }, "None");
        println!("\nSet in config.toml:\n  [shaders.by_platform]\n  {slug} = \"crt/crt-geom\"");
        return Ok(());
    }

    println!("{:<17} {:<11} shader", "platform", "display");
    for p in store.platforms()? {
        let d = match shaders::display_of(&p.fs_slug) {
            shaders::Display::Crt => "CRT",
            shaders::Display::Handheld => "handheld",
        };
        let cur = shaders::preset_for(&cfg.shaders.by_platform, &p.fs_slug);
        let shown = match &cur {
            Some(x) if shaders::resolve(&ra, x).is_some() => shaders::label_of(x).to_owned(),
            Some(x) => format!("{x} (MISSING)"),
            None => "none".to_owned(),
        };
        println!("  {:<15} {:<11} {}", p.fs_slug, d, shown);
    }
    println!("\n`shaders <platform>` lists the alternatives.");
    Ok(())
}

/// Download and install RetroArch itself.
async fn cmd_install_retroarch(version: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    if let Ok(existing) = locate_retroarch(&cfg) {
        println!("already installed: {}", existing.root.display());
        println!("(installing again would add a second entry; remove it first if that is what you want)");
        return Ok(());
    }

    let http = moose_rack::util::http_client(None)?;
    let version = match version {
        Some(v) => v.to_owned(),
        None => retroarch_install::latest_available(&http).await?,
    };
    // Alongside the rest of our data, so one folder still holds everything.
    let dest = PathBuf::from(&cfg.library.local_root).join("RetroArch");
    println!("installing RetroArch {version} into {}", dest.display());

    let started = std::time::Instant::now();
    let mut last = std::time::Instant::now();
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let root = retroarch_install::install(&http, &version, &dest, |done, total| {
        if !interactive || last.elapsed().as_millis() < 200 {
            return;
        }
        last = std::time::Instant::now();
        let pct = if total > 0 { done as f64 / total as f64 * 100.0 } else { 0.0 };
        print!("\r  {pct:5.1}%  {} / {}   ", human(done), human(total));
        use std::io::Write as _;
        std::io::stdout().flush().ok();
    })
    .await?;

    println!("\ninstalled in {:.0}s -> {}", started.elapsed().as_secs_f64(), root.display());
    println!("\nAdd it to your boot order in config.toml:");
    println!("  [[retroarch.installs]]");
    println!("  label = \"Downloaded\"");
    println!("  path = \"{}\"", root.display());
    println!("  enabled = true");
    println!("\nThen `cores --install` to fetch the libretro cores.");
    Ok(())
}

/// Stage 0.5 — install missing cores from the buildbot.
async fn cmd_cores(install: bool, update: bool) -> Result<()> {
    let cfg = Config::load()?;
    let ra = locate_retroarch(&cfg)?;
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let segment = cores::platform_segment()?;

    if update {
        return update_cores(&ra, segment).await;
    }

    // Unique cores we need but don't have, and which platforms want them.
    let mut wanted: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for (platform, core) in &map.default_core_by_server_platform {
        if !ra.has_core(core) {
            wanted.entry(core).or_default().push(platform);
        }
    }

    if wanted.is_empty() {
        println!("all {} mapped platforms have their default core installed",
                 map.default_core_by_server_platform.len());
        return Ok(());
    }

    println!("{} core(s) missing (buildbot: {segment})\n", wanted.len());
    for (core, platforms) in &wanted {
        println!("  {:<20} for {}", core, platforms.join(", "));
    }

    if !install {
        println!("\n(dry run — pass --install to download into {})", ra.cores_dir().display());
        return Ok(());
    }

    let client = moose_rack::util::http_client(None)?;
    let dest = ra.cores_dir();
    println!("\ninstalling into {}", dest.display());

    let mut ok = 0;
    let mut failed = Vec::new();
    for core in wanted.keys() {
        print!("  {core:<20} ");
        use std::io::Write as _;
        std::io::stdout().flush().ok();
        match cores::install(&client, core, &dest, segment).await {
            Ok(bytes) => {
                println!("ok ({:.1} MB)", bytes as f64 / 1_048_576.0);
                ok += 1;
            }
            Err(e) => {
                println!("FAILED — {e}");
                failed.push(*core);
            }
        }
    }

    println!("\n{ok} installed, {} failed", failed.len());
    if !failed.is_empty() {
        println!("failed: {}", failed.join(", "));
        println!("Some cores are not built for every arch; try the x86_64 buildbot under Rosetta.");
    }
    Ok(())
}

const CACHE_DB: &str = "cache.sqlite3";

/// Stage 2 — pull metadata into the local cache.
async fn cmd_sync(full: bool) -> Result<()> {
    let cfg = Config::load()?;
    let client = cfg.server.client()?;
    let mut store = cache::Cache::open(Path::new(CACHE_DB))?;

    // Refresh the settings that govern how we hash and verify, so a server
    // config change cannot silently corrupt later verification.
    match client.config().await {
        Ok(cfg) => {
            if cfg.skip_hash_calculation {
                println!(
                    "note: server has SKIP_HASH_CALCULATION set — it stores no hashes, \n\
                     so downloads can only be size-checked."
                );
            }
            store.save_server_config(&cfg).ok();
        }
        Err(e) => eprintln!("warning: could not read /api/config ({e}); using last known values"),
    }
    if let Ok(hb) = client.heartbeat().await {
        let v = hb.system.version;
        if !v.is_empty() {
            if store.server_version().as_deref() != Some(v.as_str())
                && v != moose_rack::VERIFIED_AGAINST
            {
                eprintln!(
                    "warning: server reports version {v}, but this client's server-specific behavior\n\
                     (archive hashing, query params) was verified against {}. Re-check\n\
                     `hash-parity` and a download or two.",
                    moose_rack::VERIFIED_AGAINST
                );
            }
            store.set_server_version(&v).ok();
        }
    }

    let before = store.rom_count().unwrap_or(0);
    let started = std::time::Instant::now();
    let (platforms, upserted, incremental) = store.sync(&client, full).await?;
    // Removals never show up in an incremental pull, so reconcile against the
    // server's full id list.
    match client.rom_identifiers().await {
        Ok(ids) => match store.prune_missing(&ids) {
            Ok(n) if n > 0 => println!("pruned {n} rom(s) the server no longer has"),
            Ok(_) => {}
            Err(e) => eprintln!("warning: prune failed ({e})"),
        },
        Err(e) => eprintln!("warning: could not list server rom ids ({e}); skipped pruning"),
    }

    // Collections come from the server wholesale — they are RomM's grouping,
    // not ours, so there is nothing to merge.
    match client.all_collections().await {
        Ok(items) => match store.replace_collections(&items) {
            Ok(n) => println!("collections: {n} synced"),
            Err(e) => eprintln!("warning: storing collections failed ({e})"),
        },
        Err(e) => eprintln!("warning: could not fetch collections ({e}); kept the previous set"),
    }

    // Console pictures for the platform grid, straight from the server. Only
    // the ones not already cached, so this is a no-op after the first sync.
    match client.platforms().await {
        Ok(list) => {
            let pairs: Vec<(String, String)> =
                list.iter().map(|p| (p.slug.clone(), p.fs_slug.clone())).collect();
            match moose_rack::platformicon::ensure(&client, &cfg.media_dir(), &pairs).await {
                Ok(n) if n > 0 => println!("console pictures: {n} fetched"),
                Ok(_) => {}
                Err(e) => eprintln!("warning: console pictures failed ({e})"),
            }
        }
        Err(e) => eprintln!("warning: could not list platforms for pictures ({e})"),
    }

    // A sync rewrites names from the server, so restore the real arcade titles
    // afterwards rather than before.
    let names = moose_rack::arcade::names(Path::new("data/arcade-names.json"));
    match store.apply_arcade_names(&names) {
        Ok(n) if n > 0 => println!("arcade titles: {n} romset names replaced"),
        Ok(_) => {}
        Err(e) => eprintln!("warning: arcade renaming failed ({e})"),
    }

    let after = store.rom_count().unwrap_or(0);

    println!(
        "{} sync: {platforms} platforms, {upserted} rom rows in {:.1}s",
        if incremental { "incremental" } else { "full" },
        started.elapsed().as_secs_f64()
    );
    println!("cache now holds {after} roms ({:+})", after - before);
    if let Some(w) = store.watermark() {
        println!("watermark {w}");
    }
    Ok(())
}

/// Download every ROM of one platform, several at a time.
///
/// Sequential transfers waste most of the wall clock on per-request latency
/// when the files are small — an arcade set averages 4.5 MB.
/// Classify probe logs gathered on another machine.
///
/// The probing runs where it is cheap — a headless server, in parallel — and
/// the judging happens here, through the same `read_verdict` the local probe
/// uses. One implementation of "did it run", not two that drift.
fn cmd_probe_report(file: &str) -> Result<()> {
    use moose_rack::probe::{Verdict, core_title, read_verdict};
    use std::collections::BTreeMap;

    let store = cache::Cache::open(Path::new(CACHE_DB)).ok();
    let titles: BTreeMap<String, String> = store
        .as_ref()
        .and_then(|c| c.roms_for("arcade").ok())
        .map(|rows| rows.into_iter().map(|r| (r.fs_name, r.name)).collect())
        .unwrap_or_default();

    let text = std::fs::read_to_string(file)
        .with_context(|| format!("reading {file}"))?;

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut mislabelled = Vec::new();
    let mut broken = Vec::new();
    let mut total = 0;

    for line in text.lines() {
        let mut cols = line.split('\t');
        let (Some(rom), Some(_core), Some(log)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        total += 1;
        let log = log.replace('\u{1f}', "\n");
        let (verdict, evidence) = read_verdict(&log);
        *counts.entry(verdict.label()).or_default() += 1;

        if !verdict.ok() && verdict != Verdict::IsBios {
            broken.push((rom.to_owned(), verdict.label(), evidence));
        }
        // The core knows the game's real title. Comparing it with the library's
        // label is free here, because the run already had to read the log.
        if let (Some(real), Some(ours)) = (core_title(&log), titles.get(rom)) {
            let norm = |s: &str| {
                s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>()
            };
            if !norm(&real).starts_with(&norm(ours)) && !norm(ours).starts_with(&norm(&real)) {
                mislabelled.push((rom.to_owned(), ours.clone(), real));
            }
        }
    }

    println!("{total} games probed\n");
    for (label, n) in &counts {
        println!("  {label:24} {n:>5}");
    }

    println!("\n--- not playable ({}) ---", broken.len());
    for (rom, label, why) in broken.iter().take(40) {
        println!("  {rom:24} {label:22} {}", why.chars().take(70).collect::<String>());
    }
    if broken.len() > 40 {
        println!("  … and {} more", broken.len() - 40);
    }

    println!("\n--- label disagrees with the emulator ({}) ---", mislabelled.len());
    for (rom, ours, real) in mislabelled.iter().take(30) {
        println!("  {rom:20} yours: {:32} core: {}", ours.chars().take(30).collect::<String>(), real);
    }
    if mislabelled.len() > 30 {
        println!("  … and {} more", mislabelled.len() - 30);
    }
    Ok(())
}

/// Report artwork coverage, and what is worth trying to fix.
async fn cmd_coverage(probe: bool, sample: usize) -> Result<()> {
    use moose_rack::coverage::{self, Kind};
    use std::collections::BTreeMap;

    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let media_root = cfg.media_dir();
    let client = cfg.server.client().ok();

    let mut rows: BTreeMap<String, coverage::Row> = BTreeMap::new();
    let mut gaps: BTreeMap<String, Vec<cache::RomRow>> = BTreeMap::new();

    for rom in store.all_roms()? {
        let stem = Path::new(&rom.fs_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| rom.fs_name.clone());
        let has_art = media::ART_CHAIN
            .iter()
            .any(|k| media::find_local(&media_root, &rom.platform_slug, &stem, k).is_some());
        let row = rows.entry(rom.platform_slug.clone()).or_default();
        match coverage::classify_in(&rom.platform_slug, &rom.fs_name) {
            Kind::Official => {
                row.official += 1;
                if has_art {
                    row.official_with_art += 1;
                } else {
                    gaps.entry(rom.platform_slug.clone()).or_default().push(rom);
                }
            }
            Kind::Unofficial => {
                row.unofficial += 1;
                if has_art {
                    row.unofficial_with_art += 1;
                }
            }
        }
    }

    // Ask the server whether ScreenScraper knows each gap. A sample per
    // platform rather than all of them: the answer is a rate, and one request
    // per game across nine thousand is an hour to learn a percentage.
    if probe && let Some(client) = client.as_ref() {
        for (platform, missing) in &gaps {
            let row = rows.get_mut(platform).expect("platform seen above");
            for rom in missing.iter().take(sample) {
                let known = client
                    .identify(rom.id, &rom.name)
                    .await
                    .map(|m| m.iter().any(|c| c.ss_id.is_some() || !c.ss_url_cover.is_empty()))
                    .unwrap_or(false);
                row.probed += 1;
                if !known {
                    row.not_in_database += 1;
                }
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
            // Scale the sample up to the whole gap, so the estimate is about
            // the platform rather than about the first forty names in it.
            let gap = row.official - row.official_with_art;
            if row.probed > 0 && gap > row.probed {
                row.not_in_database = row.not_in_database * gap / row.probed;
            }
            println!("  probed {platform}…");
        }
        println!();
    }

    println!(
        "{:16}{:>8}{:>9}{:>10}{:>12}{:>11}",
        "platform", "released", "with art", "coverage", "no entry", "scrapeable"
    );
    for (platform, r) in &rows {
        println!(
            "{:16}{:>8}{:>9}{:>9.0}%{:>12}{:>11}",
            platform,
            r.official,
            r.official_with_art,
            r.percent(),
            if probe { r.not_in_database.to_string() } else { "-".to_owned() },
            if probe { r.worth_scraping().to_string() } else { "-".to_owned() },
        );
    }
    let t = coverage::totals(&rows);
    println!(
        "\n{:16}{:>8}{:>9}{:>9.0}%{:>12}{:>11}",
        "TOTAL",
        t.official,
        t.official_with_art,
        t.percent(),
        if probe { t.not_in_database.to_string() } else { "-".to_owned() },
        if probe { t.worth_scraping().to_string() } else { "-".to_owned() },
    );
    println!(
        "\n{} hacks, prototypes and unlicensed dumps excluded ({} of them have art anyway)",
        t.unofficial, t.unofficial_with_art
    );
    Ok(())
}

/// Fill in artwork for games ES-DE never scraped.
///
/// One at a time and unhurried. ScreenScraper throttles by account tier and
/// answers an exceeded allowance with a rejection rather than a picture, so a
/// run that hammers it finishes quickly and fetches nothing — and the account
/// being spent is the server's, shared with anything else pointed at it.
async fn cmd_scrape(
    platform: Option<&str>,
    game: Option<&str>,
    limit: Option<usize>,
    videos: bool,
    dry_run: bool,
) -> Result<()> {
    use moose_rack::scrape;

    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let client = cfg.server.client()?;
    let media_root = cfg.media_dir();

    let mut todo = scrape::missing(&store, &media_root, platform)?;
    // A single game is asked for by name, and the point of asking is usually to
    // see whether the route works at all — so it is *not* filtered by whether
    // the game already has artwork. Re-fetching one on purpose is the whole
    // request.
    if let Some(term) = game {
        let want = term.to_lowercase();
        let rows = match platform {
            Some(p) => store.roms_for(p)?,
            None => store.all_roms()?,
        };
        todo = rows.into_iter().filter(|r| r.name.to_lowercase().contains(&want)).collect();
        if todo.is_empty() {
            bail!("no game matches {term:?}");
        }
    }
    let total_missing = todo.len();
    if let Some(n) = limit {
        todo.truncate(n);
    }

    println!(
        "{total_missing} game(s) with no artwork{}{}",
        platform.map(|p| format!(" on {p}")).unwrap_or_default(),
        if todo.len() < total_missing { format!("; doing {}", todo.len()) } else { String::new() },
    );
    if dry_run {
        for row in todo.iter().take(40) {
            println!("  {} [{}]", row.name, row.platform_slug);
        }
        if todo.len() > 40 {
            println!("  … and {} more", todo.len() - 40);
        }
        return Ok(());
    }

    let mut report = scrape::Report::default();
    let started = std::time::Instant::now();
    for (i, row) in todo.iter().enumerate() {
        if let Err(e) = scrape::fill_one(&client, &media_root, row, videos, &mut report).await {
            eprintln!("  {}: {e}", row.name);
        }
        if (i + 1).is_multiple_of(25) || i + 1 == todo.len() {
            println!("  {}/{} in {:.0}s", i + 1, todo.len(), started.elapsed().as_secs_f64());
        }
        // Deliberate. See the note above about whose allowance this is.
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    for platform in todo.iter().map(|r| r.platform_slug.as_str()).collect::<std::collections::BTreeSet<_>>() {
        moose_rack::media::clear_art_index(&media_root, platform);
    }
    println!("{}", report.describe());
    Ok(())
}

async fn cmd_get_platform(slug: &str, jobs: usize) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    moose_rack::apply_cached_server_config(&store);
    let rows = store.roms_for(slug)?;
    if rows.is_empty() {
        bail!("no cached roms for platform {slug:?} — try `sync` first");
    }
    cmd_get_bulk(rows, jobs, &cfg).await
}

/// Everything in a collection group — `user` is what the app calls
/// "My collections".
///
/// A group rather than one collection because the lists overlap heavily and
/// picking them off one at a time would mean re-deciding, per list, what has
/// already been fetched.
async fn cmd_get_group(grp: &str, jobs: usize) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    moose_rack::apply_cached_server_config(&store);
    let rows = store.roms_in_group(grp)?;
    if rows.is_empty() {
        bail!("no cached games in collection group {grp:?} — try `sync` first");
    }
    cmd_get_bulk(rows, jobs, &cfg).await
}

/// Fetch a list of games concurrently, reporting as it goes.
///
/// Already-complete files are settled by `download::fetch` itself, which
/// verifies before transferring, so re-running this is cheap and is the way to
/// resume an interrupted run.
async fn cmd_get_bulk(rows: Vec<cache::RomRow>, jobs: usize, cfg: &Config) -> Result<()> {
    use futures_util::StreamExt as _;

    let roms_dir = cfg.local_roms_dir();
    let client = std::sync::Arc::new(cfg.server.client()?);

    let total: i64 = rows.iter().map(|r| r.fs_size_bytes.max(0)).sum();
    println!("{} games, {} — {jobs} at a time", rows.len(), human(total as u64));
    let started = std::time::Instant::now();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let count = rows.len();

    futures_util::stream::iter(rows.into_iter().map(|rom| {
        let client = client.clone();
        let roms_dir = roms_dir.clone();
        let done = done.clone();
        let failed = failed.clone();
        async move {
            let members = if rom.multi_file {
                client.member_hashes(rom.id).await
            } else {
                Vec::new()
            };
            let target = download::Target {
                rom_id: rom.id,
                members: &members,
                fs_name: &rom.fs_name,
                platform_slug: &rom.platform_slug,
                expected_size: (rom.fs_size_bytes > 0).then_some(rom.fs_size_bytes as u64),
                md5: rom.md5_hash.as_deref(),
                sha1: rom.sha1_hash.as_deref(),
                multi_file: rom.multi_file,
            };
            let r = download::fetch(
                client.http(),
                client.base(),
                client.auth(),
                &target,
                &roms_dir,
                |_, _| {},
            )
            .await;
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if let Err(e) = r {
                failed.lock().unwrap().push(format!("{}: {e}", rom.fs_name));
            }
            if n.is_multiple_of(25) || n == count {
                let secs = started.elapsed().as_secs_f64();
                println!("  {n}/{count} in {secs:.0}s");
            }
        }
    }))
    .buffer_unordered(jobs.max(1))
    .collect::<Vec<_>>()
    .await;

    let failed = failed.lock().unwrap();
    println!(
        "\n{}/{count} downloaded in {:.0}s",
        count - failed.len(),
        started.elapsed().as_secs_f64()
    );
    for f in failed.iter().take(15) {
        println!("  failed: {f}");
    }
    if failed.len() > 15 {
        println!("  … and {} more", failed.len() - 15);
    }
    Ok(())
}

/// Find out which cores actually run something, by running them.
///
/// Downloads what it needs first: a verdict about a game we do not have is
/// worthless, and arcade ROMs are small.
async fn cmd_probe(
    term: Option<&str>,
    platform: Option<&str>,
    sample: usize,
    cores: Option<&str>,
    frames: u32,
) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let ra = locate_retroarch(&cfg)?;
    let scratch = probe::scratch_dir(&cfg.library.local_root);

    let roms = match (term, platform) {
        (Some(t), _) => {
            let found = if let Ok(id) = t.parse::<i64>() {
                store.rom_by_id(id)?.into_iter().collect()
            } else {
                store.search(t, 5)?
            };
            if found.is_empty() {
                bail!("nothing matches {t:?}");
            }
            found
        }
        (None, Some(p)) => {
            let mut all = store.roms_for(p)?;
            if all.is_empty() {
                bail!("no cached roms for platform {p:?}");
            }
            // Spread the sample across the alphabet rather than taking the
            // first N, which on this library would be all numeric titles.
            let step = (all.len() / sample.max(1)).max(1);
            all = all.into_iter().step_by(step).take(sample).collect();
            all
        }
        (None, None) => bail!("give a game, or --platform <slug>"),
    };

    let slug = roms[0].platform_slug.clone();
    let candidates: Vec<String> = match cores {
        Some(list) => list.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
        None => {
            let mut v: Vec<String> = map.alternatives(&slug).into_iter().map(str::to_owned).collect();
            if let Some(d) = map.default_core(&slug)
                && !v.iter().any(|c| c == d)
            {
                v.insert(0, d.to_owned());
            }
            v
        }
    };
    if candidates.is_empty() {
        bail!("no candidate cores for platform {slug:?}; pass --cores");
    }

    println!(
        "probing {} game(s) against {}: {}\n",
        roms.len(),
        candidates.len(),
        candidates.join(", ")
    );

    let client = cfg.server.client().ok();
    let roms_dir = cfg.local_roms_dir();
    let mut tally: std::collections::BTreeMap<String, (usize, usize)> = Default::default();

    for rom in &roms {
        let local = roms_dir.join(&rom.platform_slug).join(&rom.fs_name);
        if !local.exists() {
            let Some(client) = client.as_ref() else {
                println!("{:<44} skipped (not downloaded, no server)", short(&rom.name));
                continue;
            };
            let members = if rom.multi_file { client.member_hashes(rom.id).await } else { Vec::new() };
            let target = download::Target {
                rom_id: rom.id,
                members: &members,
                fs_name: &rom.fs_name,
                platform_slug: &rom.platform_slug,
                expected_size: (rom.fs_size_bytes > 0).then_some(rom.fs_size_bytes as u64),
                md5: rom.md5_hash.as_deref(),
                sha1: rom.sha1_hash.as_deref(),
                multi_file: rom.multi_file,
            };
            if let Err(e) =
                download::fetch(client.http(), client.base(), client.auth(), &target, &roms_dir, |_, _| {})
                    .await
            {
                println!("{:<44} skipped ({e})", short(&rom.name));
                continue;
            }
        }
        // Neo Geo and similar need their BIOS in RetroArch's system dir, or
        // every core refuses the content for a reason unrelated to the core.
        if let Some(d) = local.parent() {
            let _ = ra.install_bios(d);
        }

        let results = probe::probe_cores(&ra, &local, &candidates, frames, &scratch)?;
        let marks: Vec<String> = results
            .iter()
            .map(|r| format!("{:>7}", if r.verdict.ok() { "ok" } else { "-" }))
            .collect();
        println!("{:<44}{}", short(&rom.name), marks.join(""));
        for r in &results {
            let e = tally.entry(r.core.clone()).or_default();
            e.1 += 1;
            if r.verdict.ok() {
                e.0 += 1;
            }
        }
    }

    println!("\n{:<22}{:>8}  share", "core", "ran");
    let mut ranked: Vec<_> = tally.into_iter().collect();
    ranked.sort_by_key(|(_, (ok, _))| std::cmp::Reverse(*ok));
    for (core, (ok, total)) in &ranked {
        println!(
            "{core:<22}{:>8}  {:.0}%",
            format!("{ok}/{total}"),
            if *total > 0 { *ok as f64 / *total as f64 * 100.0 } else { 0.0 }
        );
    }
    println!("\ncolumns are the cores in the order listed above");
    Ok(())
}

fn short(s: &str) -> String {
    s.chars().take(42).collect()
}

/// Re-download every installed core at the current nightly.
///
/// Each existing library is copied aside first. Core updates are not always
/// improvements: MAME-family and FBNeo cores are tied to particular romset
/// vintages, and a newer build can stop accepting a set that works today. A
/// rollback copy makes that recoverable instead of a re-download-and-hope.
async fn update_cores(ra: &RetroArch, segment: &str) -> Result<()> {
    let installed = ra.installed_cores();
    if installed.is_empty() {
        bail!("no cores installed at {}", ra.cores_dir().display());
    }

    let dest = ra.cores_dir();
    let backup = dest.parent().unwrap_or(&dest).join("cores-previous");
    std::fs::create_dir_all(&backup)?;
    println!(
        "updating {} core(s) from {segment}\n  into    {}\n  rollback {}\n",
        installed.len(),
        dest.display(),
        backup.display()
    );

    let client = moose_rack::util::http_client(Some(std::time::Duration::from_secs(180)))?;

    let mut updated = Vec::new();
    let mut unchanged = 0;
    let mut failed = Vec::new();

    for core in &installed {
        print!("  {core:<22} ");
        use std::io::Write as _;
        std::io::stdout().flush().ok();

        let path = ra.core_path(core);
        let before = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // Keep the working copy until the replacement is on disk.
        let saved = backup.join(path.file_name().unwrap_or_default());
        std::fs::copy(&path, &saved).ok();

        match cores::install(&client, core, &dest, segment).await {
            Ok(_) => {
                let after = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if after == before {
                    println!("unchanged");
                    unchanged += 1;
                } else {
                    let d = after as i64 - before as i64;
                    println!("updated ({:+.0} KB)", d as f64 / 1024.0);
                    updated.push(core.clone());
                }
            }
            Err(e) => {
                // Put the working core back; a failed update must not leave a
                // hole where a core used to be.
                std::fs::copy(&saved, &path).ok();
                println!("FAILED ({})", e.to_string().lines().next().unwrap_or(""));
                failed.push(core.clone());
            }
        }
    }

    println!(
        "\n{} updated, {unchanged} already current, {} failed",
        updated.len(),
        failed.len()
    );
    if !failed.is_empty() {
        println!("failed (previous version kept): {}", failed.join(", "));
    }
    let risky: Vec<&String> = updated
        .iter()
        .filter(|c| c.starts_with("mame") || c.starts_with("fbneo") || c.contains("neo"))
        .collect();
    if !risky.is_empty() {
        println!(
            "\nArcade cores changed: {}.\n\
             Romsets are version-locked, so re-check a few arcade games. To roll\n\
             back, copy the file(s) from {} back over {}.",
            risky.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            backup.display(),
            dest.display()
        );
    }
    Ok(())
}

/// Index a local ES-DE library into the cache.
fn cmd_scan_esde(root: Option<&str>, roms: Option<&str>, media: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    let layout = match root {
        Some(r) => moose_rack::esde::Layout::new(
            &moose_rack::util::expand_tilde(r),
            roms.map(moose_rack::util::expand_tilde).as_deref(),
        )
        .with_media(media.map(moose_rack::util::expand_tilde).as_deref()),
        // No --root: the configured library, or this install's own folders,
        // which have the same shape. There is always somewhere to look.
        None => cfg.esde_layout(),
    };

    println!("roms       {}", layout.roms.display());
    println!("gamelists  {}", layout.gamelists.display());
    println!("media      {}", layout.media.display());

    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let started = std::time::Instant::now();
    let mut store = cache::Cache::open(Path::new(CACHE_DB))?;
    // Shared with the service, fold included -- see `moose_rack::app::scan_into`.
    let moose_rack::app::Scan { written: n, folded, games, skipped } =
        moose_rack::app::scan_into(&mut store, &layout, &map)?;
    if games.is_empty() {
        bail!("found no games under {}", layout.roms.display());
    }

    let mut by_platform: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut with_art = 0;
    for g in &games {
        *by_platform.entry(g.platform_slug.as_str()).or_default() += 1;
        if moose_rack::esde::media_path(&layout, &g.system, 
            g.fs_name.rsplit_once('.').map_or(g.fs_name.as_str(), |(s, _)| s), "covers").is_some() {
            with_art += 1;
        }
    }
    println!("\n{n} games in {:.1}s, {with_art} with cover art", started.elapsed().as_secs_f64());
    if folded > 0 {
        println!("{folded} folded into rows the server already had — they launch from disk now\n");
    } else {
        println!();
    }
    println!("{:<16}{:>7}", "platform", "games");
    for (slug, count) in &by_platform {
        println!("{slug:<16}{count:>7}");
    }
    if !skipped.is_empty() {
        println!("\nskipped (no platform mapping): {}", skipped.join(", "));
    }
    Ok(())
}

/// Show the collection groups, or what is inside one.
fn cmd_collections(group: Option<&str>) -> Result<()> {
    let store = cache::Cache::open(Path::new(CACHE_DB))?;

    let Some(group) = group else {
        let groups = store.collection_groups()?;
        if groups.is_empty() {
            bail!("no collections cached — run `sync` first");
        }
        println!("{:<14} {:>6}", "group", "count");
        for (name, n) in &groups {
            println!("{name:<14} {n:>6}");
        }
        println!("\n{} collections total", store.collection_count()?);
        return Ok(());
    };

    let items = store.collections_in(group)?;
    if items.is_empty() {
        bail!("no collections in group {group:?} — try `collections` for the list");
    }
    println!("{:<46} {:>6}", format!("{group} collections"), "games");
    for c in items.iter().take(60) {
        println!(
            "{:<46} {:>6}",
            c.name.chars().take(44).collect::<String>(),
            c.rom_count
        );
    }
    if items.len() > 60 {
        println!("… and {} more", items.len() - 60);
    }
    Ok(())
}

/// Stage 4 — download a ROM by search term.
async fn cmd_get(needle: &str) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    moose_rack::apply_cached_server_config(&store);
    // A bare number is an id: with several platforms carrying identically
    // named folders, a search term alone cannot always name one ROM.
    if let Ok(id) = needle.parse::<i64>()
        && let Some(rom) = store.rom_by_id(id)?
    {
        return download_one(rom, &cfg).await;
    }

    let matches = store.search(needle, 25)?;

    let rom = match matches.len() {
        0 => bail!("nothing in the cache matches {needle:?} — try `sync` first"),
        1 => matches.into_iter().next().unwrap(),
        n => {
            println!("{n} matches — be more specific:");
            for m in matches.iter().take(15) {
                println!(
                    "  {:<16} {:<48} {:>10}",
                    m.platform_slug,
                    m.name.chars().take(46).collect::<String>(),
                    human(m.fs_size_bytes as u64)
                );
            }
            return Ok(());
        }
    };

    download_one(rom, &cfg).await
}

/// Fetch one resolved ROM, reporting progress and how it was verified.
async fn download_one(rom: cache::RomRow, cfg: &Config) -> Result<()> {
    let client = cfg.server.client()?;
    let roms_dir = cfg.local_roms_dir();

    println!(
        "{} [{}]  {}",
        rom.name,
        rom.platform_slug,
        human(rom.fs_size_bytes as u64)
    );

    // Folder ROMs verify per member; the rom-level hash is not reproducible.
    let members = if rom.multi_file {
        client.member_hashes(rom.id).await
    } else {
        Vec::new()
    };

    let target = download::Target {
        rom_id: rom.id,
        members: &members,
        fs_name: &rom.fs_name,
        platform_slug: &rom.platform_slug,
        expected_size: (rom.fs_size_bytes > 0).then_some(rom.fs_size_bytes as u64),
        md5: rom.md5_hash.as_deref(),
        sha1: rom.sha1_hash.as_deref(),
        multi_file: rom.multi_file,
    };

    let started = std::time::Instant::now();
    let mut last_tick = std::time::Instant::now();
    // Bytes already on disk when we started, so the rate reflects what this
    // session actually transferred rather than crediting us with the resume.
    let mut baseline: Option<u64> = None;
    // Carriage-return progress only makes sense on a terminal; piped or
    // redirected it produces thousands of lines of noise.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let outcome = download::fetch(
        client.http(),
        client.base(),
        client.auth(),
        &target,
        &roms_dir,
        |done, total| {
            let base = *baseline.get_or_insert(done);
            if !interactive {
                return;
            }
            // Throttle so the terminal isn't the bottleneck on a fast transfer.
            if last_tick.elapsed().as_millis() < 200 {
                return;
            }
            last_tick = std::time::Instant::now();
            let pct = if total > 0 {
                format!("{:5.1}%", done as f64 / total as f64 * 100.0)
            } else {
                "  ?  ".to_owned()
            };
            let rate = done.saturating_sub(base) as f64
                / started.elapsed().as_secs_f64().max(0.001)
                / 1_048_576.0;
            print!(
                "\r  {pct}  {:>10} / {:<10} {rate:6.1} MB/s   ",
                human(done),
                human(total)
            );
            use std::io::Write as _;
            std::io::stdout().flush().ok();
        },
    )
    .await;

    println!();
    match outcome? {
        download::Outcome::AlreadyHave(p) => println!("already downloaded and verified: {}", p.display()),
        download::Outcome::Downloaded { path, bytes, resumed_from, verified } => {
            let how = verified.describe();
            if resumed_from > 0 {
                println!("resumed from {}", human(resumed_from));
            }
            println!(
                "{} in {:.1}s ({})\n{}",
                human(bytes),
                started.elapsed().as_secs_f64(),
                how,
                path.display()
            );
        }
    }
    Ok(())
}

/// Stage 5a — report what the local save scanner finds.
fn cmd_scan() -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    if store.rom_count().unwrap_or(0) == 0 {
        bail!("cache is empty — run `sync` first so saves can be resolved to rom ids");
    }
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let root = Path::new(&cfg.saves.root);
    if !root.is_dir() {
        bail!("{} not found — set [saves] root in config.toml", root.display());
    }

    let found = saves::scan(root, &store, &map)?;
    println!("scanning {}\n", root.display());

    let (mut ok, mut amb, mut unmatched, mut unknown, mut superseded) = (0, 0, 0, 0, 0);
    for c in &found {
        let kind = match c.kind { saves::Kind::Save => "save ", saves::Kind::State => "state" };
        let core = c.core.clone().unwrap_or_else(|| format!("?{}", c.core_dir));
        print!("{kind} {:<10} {:<9} {:<44}", core, c.slot, c.rom_base.chars().take(42).collect::<String>());
        match &c.resolution {
            saves::Resolution::Resolved { rom_id, platform, .. } => {
                if c.canonical {
                    ok += 1;
                    println!(" -> {platform}/{rom_id}");
                } else {
                    superseded += 1;
                    println!(
                        " -> {platform}/{rom_id}  SUPERSEDED by {}",
                        c.superseded_by.as_deref().unwrap_or("?")
                    );
                }
            }
            saves::Resolution::Ambiguous(hits) => {
                amb += 1;
                println!(" -> AMBIGUOUS: {}", hits.iter()
                    .map(|(id, p, _)| format!("{p}/{id}"))
                    .collect::<Vec<_>>().join(", "));
            }
            saves::Resolution::Unmatched => { unmatched += 1; println!(" -> no matching rom"); }
            saves::Resolution::UnknownCore => { unknown += 1; println!(" -> unknown core dir"); }
        }
    }

    println!("\n{} files: {ok} to sync, {superseded} superseded, {amb} ambiguous, \
              {unmatched} unmatched, {unknown} unknown core", found.len());
    if superseded > 0 {
        println!("\nSuperseded entries share a (rom_id, slot) with a save from the platform's\n\
                  default core. Only the default core's file is synced, or they would\n\
                  overwrite each other on the server every run.");
    }
    if amb > 0 {
        println!("\nAmbiguous entries are NOT guessed — the same filename exists on more than one\n\
                  platform (this library has arcade+mame and nes+famicom overlapping).");
    }
    Ok(())
}

/// Resolve artwork for a ROM — local first, else fetch from the server.
async fn cmd_art(needle: &str) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let client = cfg.server.client().ok();
    let media_root = PathBuf::from(&cfg.library.local_root).join("downloaded_media");

    let matches = store.search(needle, 5)?;
    if matches.is_empty() {
        bail!("nothing matches {needle:?}");
    }
    for rom in matches {
        let stem = Path::new(&rom.fs_name)
            .file_stem().map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| rom.fs_name.clone());
        println!("{} [{}]", rom.name, rom.platform_slug);
        for (kind, _) in media::ESDE_TYPES {
            let before =
                media::find_local(&media_root, &rom.platform_slug, &stem, kind).is_some();
            // Every type comes from the ES-DE tree now, covers included: the
            // app no longer reads RomM's own copies, so reporting them here
            // would list artwork it will never show.
            let got = media::ensure_esde(
                client.as_ref(), &media_root, &rom.platform_slug, &stem, kind,
            ).await;
            let how = match (&got, before) {
                (Some(_), true) => "local",
                (Some(_), false) => "FETCHED",
                (None, _) => "none",
            };
            println!("  {kind:<15} {how:<8} {}",
                     got.map(|p| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())
                        .unwrap_or_default());
        }
    }
    Ok(())
}

/// Inspect ES-DE themes and install their system logos locally.
fn cmd_themes(install: bool) -> Result<()> {
    let cfg = Config::load()?;
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let slugs: Vec<String> = store.platforms()?.into_iter().map(|p| p.fs_slug).collect();

    let themes = theme::discover_with(cfg.theme.root.as_deref(), Some(&cfg.themes_dir()));
    if themes.is_empty() {
        bail!("no ES-DE themes found — install ES-DE, or set [theme] root in config.toml");
    }
    println!("themes found:");
    for t in &themes {
        println!("  {:<16} {}", t.name, t.path.display());
    }

    let found = theme::logos(&themes, &map, &slugs);
    println!("\nlogos: {}/{} platforms", found.len(), slugs.len());
    for slug in &slugs {
        match found.get(slug) {
            Some(p) => println!("  {slug:<16} {}", p.display()),
            None => println!("  {slug:<16} — none"),
        }
    }

    if install {
        let media_root = cfg.media_dir();
        let n = theme::install(&themes, &map, &slugs, &media_root)?;
        println!("\ninstalled {n} logos into {}/_platforms", media_root.display());
    } else {
        println!("\n(pass --install to copy them into the local media tree)");
    }
    Ok(())
}

/// List themes available from the official ES-DE themes list.
async fn cmd_themes_available(filter: Option<&str>) -> Result<()> {
    let http = moose_rack::util::http_client(None)?;
    let mut list = theme_remote::list(&http).await?;
    if let Some(f) = filter {
        list.retain(|t| t.matches(f));
    }
    let cfg = Config::load()?;
    let dir = cfg.themes_dir();

    println!("{} themes available\n", list.len());
    for t in &list {
        let installed = dir.join(t.dir_name()).is_dir();
        println!(
            "  {}{:<26} {:<28} {}",
            if installed { "* " } else { "  " },
            t.name.chars().take(24).collect::<String>(),
            t.dir_name(),
            t.author
        );
    }
    println!("\n* = already downloaded    (cargo run -- themes --get <name>)");
    Ok(())
}

/// Download (or update) a theme from the official list.
async fn cmd_themes_get(needle: &str, logos_only: bool) -> Result<()> {
    let cfg = Config::load()?;
    let http = moose_rack::util::http_client(None)?;
    let list = theme_remote::list(&http).await?;

    let hits: Vec<_> = list.iter().filter(|t| t.matches(needle)).collect();
    let theme_entry = match hits.len() {
        0 => bail!("no theme matches {needle:?} — try `themes --available`"),
        1 => hits[0],
        _ => {
            // An exact name match disambiguates; otherwise make the user choose.
            match hits.iter().find(|t| t.name.eq_ignore_ascii_case(needle)) {
                Some(t) => t,
                None => {
                    println!("{} matches — be more specific:", hits.len());
                    for t in hits {
                        println!("  {:<26} {}", t.name, t.dir_name());
                    }
                    return Ok(());
                }
            }
        }
    };

    let dir = cfg.themes_dir();
    println!("{} — {}", theme_entry.name, theme_entry.url);
    println!("downloading into {} …", dir.join(theme_entry.dir_name()).display());
    let (path, fresh) = theme_remote::install(&http, theme_entry, &dir).await?;
    let size = theme_remote::size_of(&path);
    println!(
        "{} ({:.1} MB)",
        if fresh { "downloaded" } else { "updated" },
        size as f64 / 1_048_576.0
    );

    if logos_only {
        // Themes ship hundreds of MB of wallpapers and per-system art we never
        // render. Keep the logos, drop the rest.
        let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
        let store = cache::Cache::open(Path::new(CACHE_DB))?;
        let slugs: Vec<String> =
            store.platforms()?.into_iter().map(|p| p.fs_slug).collect();
        let one = vec![moose_rack::theme::Theme {
            name: theme_entry.dir_name(),
            path: path.clone(),
        }];
        let n = theme::install(&one, &map, &slugs, &cfg.media_dir())?;
        theme_remote::remove(&theme_entry.dir_name(), &dir)?;
        println!(
            "kept {n} logos, deleted the {:.0} MB checkout",
            size as f64 / 1_048_576.0
        );
        return Ok(());
    }

    println!("\nRun `themes --install` to copy its logos into the platform grid.");
    println!("(or re-run with --logos-only to keep just the icons and delete the rest)");
    Ok(())
}

/// Update every downloaded theme.
async fn cmd_themes_update() -> Result<()> {
    let cfg = Config::load()?;
    let dir = cfg.themes_dir();
    let http = moose_rack::util::http_client(None)?;
    let list = theme_remote::list(&http).await?;

    let Ok(entries) = std::fs::read_dir(&dir) else {
        bail!("nothing downloaded yet — see `themes --available`");
    };
    let mut n = 0;
    for entry in entries.flatten().filter(|e| e.path().is_dir()) {
        let name = entry.file_name().to_string_lossy().to_string();
        match list.iter().find(|t| t.dir_name() == name) {
            Some(t) => {
                print!("  {name:<28} ");
                use std::io::Write as _;
                std::io::stdout().flush().ok();
                match theme_remote::install(&http, t, &dir).await {
                    Ok(_) => { println!("ok"); n += 1; }
                    Err(e) => println!("FAILED — {e}"),
                }
            }
            None => println!("  {name:<28} not in the official list, skipped"),
        }
    }
    println!("\n{n} theme(s) updated");
    Ok(())
}

/// Debug: show how a file hashes, as a whole and as an archive member.
fn cmd_hashcheck(path: &Path) -> Result<()> {
    let (md5, sha1) = download::hash_file(path)?;
    println!("file        md5 {md5}");
    println!("            sha1 {sha1}");
    if let Some((m, s)) = download::hash_archive_composite(path) {
        println!("composite   md5 {m}");
        println!("            sha1 {s}");
    }
    for (name, md5, sha1) in download::hash_archive_members(path) {
        println!("member      {name}");
        println!("            md5 {md5}");
        println!("            sha1 {sha1}");
    }
    Ok(())
}

/// Stage 2 — browse the cache.
fn cmd_browse() -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    if store.rom_count().unwrap_or(0) == 0 {
        bail!("cache is empty — run `cargo run -- sync` first");
    }
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    // Missing RetroArch is not fatal; browsing still works, launching doesn't.
    let ra = locate_retroarch(&cfg).ok();
    // Likewise a missing server only disables downloading.
    let client = cfg.server.client()
        .ok()
        .map(std::sync::Arc::new);
    tui::run(
        &store,
        &cfg.local_roms_dir(),
        ra,
        map,
        client,
        tokio::runtime::Handle::current(),
    )
}

/// Frontends over one library: this CLI/TUI, and the Tauri GUI in `src-tauri/`.
#[derive(Parser)]
#[command(
    name = "moose-rack",
    about = "Browse, download and launch a self-hosted RomM library",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify server auth and that the library is reachable
    Check,
    /// Pull metadata into the local cache
    Sync {
        /// Ignore the watermark and re-fetch everything
        #[arg(long)]
        full: bool,
    },
    /// Terminal library browser
    Browse,
    /// Test which cores actually run a game, or a sample of a platform.
    ///
    /// OPENS A RETROARCH WINDOW PER PROBE. On macOS `video_driver = "null"`
    /// silences rendering but does NOT stop the window appearing, so N games x
    /// M cores means N*M windows. Requires --i-know-it-opens-windows.
    Probe {
        /// ROM id, or a search term
        term: Option<String>,
        /// Probe a sample of this platform instead of one game
        #[arg(long)]
        platform: Option<String>,
        /// How many games to sample from the platform
        #[arg(long, default_value_t = 12)]
        sample: usize,
        /// Cores to try, comma separated. Defaults to every core the map
        /// knows for the platform.
        #[arg(long)]
        cores: Option<String>,
        /// Frames to run before exiting. 180 is about three seconds.
        #[arg(long, default_value_t = 180)]
        frames: u32,
        /// Required. Each probe opens a RetroArch window; there is no headless
        /// mode on macOS.
        #[arg(long)]
        i_know_it_opens_windows: bool,
    },
    /// Index a local ES-DE library instead of a RomM server
    ScanEsde {
        /// ES-DE data directory (holds gamelists/ and downloaded_media/)
        #[arg(long)]
        root: Option<String>,
        /// ROMs directory, if not <root>/ROMs
        #[arg(long)]
        roms: Option<String>,
        /// Artwork directory, if not <root>/downloaded_media (portable installs)
        #[arg(long)]
        media: Option<String>,
    },
    /// List collections mirrored from the server
    Collections {
        /// Show the collections inside one group, e.g. `genre`
        group: Option<String>,
    },
    /// Download a ROM (resumable, hash-verified)
    Get {
        /// Name or filename to search for
        term: Option<String>,
        /// Download an entire platform instead of one game
        #[arg(long)]
        platform: Option<String>,
        /// Download every game in a collection group. `user` is the app's
        /// "My collections"; overlapping lists are fetched once.
        #[arg(long)]
        collections: Option<String>,
        /// Concurrent transfers when fetching a whole platform
        #[arg(long, default_value_t = 6)]
        jobs: usize,
    },
    /// Fetch artwork for games that have none, through the server's own
    /// ScreenScraper account
    Scrape {
        /// One platform, e.g. `megadrive`. Omit for the whole library.
        #[arg(long)]
        platform: Option<String>,
        /// A single game, by name. For trying the route before a long run.
        #[arg(long)]
        game: Option<String>,
        /// Stop after this many, for a look before committing to a long run.
        #[arg(long)]
        limit: Option<usize>,
        /// Also fetch the gameplay video. Off by default: a video is tens of
        /// megabytes against tens of kilobytes for every picture.
        #[arg(long)]
        videos: bool,
        /// List what would be fetched and fetch nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Artwork coverage, separating gaps that can be filled from dumps that
    /// could never have art
    Coverage {
        /// Ask the server whether ScreenScraper has an entry for each gap.
        /// Slow — one request per game — so it takes a limit.
        #[arg(long)]
        probe: bool,
        /// How many games to probe per platform.
        #[arg(long, default_value_t = 40)]
        sample: usize,
    },
    /// Classify probe logs collected elsewhere (see
    /// scripts/probe-arcade-remote.sh), using the same rules as `probe`
    ProbeReport {
        /// TSV of `romset<TAB>core<TAB>log`, newlines encoded as 0x1f.
        file: String,
    },
    /// Check our content_hash implementation against the server's
    HashParity,
    /// Inspect local saves and states
    Scan,
    /// ES-DE theme logos for the console grid
    Themes {
        /// Copy logos from installed themes into the media tree
        #[arg(long)]
        install: bool,
        /// List downloadable themes, optionally filtered
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        available: Option<String>,
        /// Download or update one theme by name
        #[arg(long, value_name = "NAME")]
        get: Option<String>,
        /// With --get: keep only the platform icons and delete the checkout
        #[arg(long)]
        logos_only: bool,
        /// Update every downloaded theme
        #[arg(long)]
        update: bool,
    },
    /// Show what is installed and which platforms can launch
    Doctor,
    /// Check config.toml against what the app reads now, and offer to update it
    Config {
        /// Apply the changes that follow from the old values with no guessing
        #[arg(long)]
        fix: bool,
    },
    /// Show or list per-platform video shaders
    Shaders {
        /// Limit to one platform and list its alternatives
        platform: Option<String>,
    },
    /// Download and install RetroArch itself
    InstallRetroarch {
        /// Pin a release instead of taking the newest known one
        #[arg(long)]
        version: Option<String>,
    },
    /// List or install missing libretro cores
    Cores {
        /// Download the missing cores from the libretro buildbot
        #[arg(long)]
        install: bool,
        /// Re-download every installed core at the current nightly
        #[arg(long)]
        update: bool,
    },
    /// List one launchable ROM per platform
    Suggest,
    /// Resolve a ROM's core and print the launch command
    Launch {
        rom: PathBuf,
        /// Override the core rather than resolving one
        #[arg(long, value_name = "NAME")]
        core: Option<String>,
        /// Launch fullscreen instead of windowed
        #[arg(long)]
        fullscreen: bool,
        /// Actually spawn the emulator; without this it is a dry run
        #[arg(long)]
        go: bool,
        /// The controller name, as RetroArch's autoconfig knows it. The pad
        /// decides the hotkey and rapid-fire button numbers, so a dry run
        /// without one writes neither — which is why "rapid fire does
        /// nothing" could not be told apart from "rapid fire was never
        /// written" from a terminal.
        #[arg(long, value_name = "NAME")]
        pad: Option<String>,
    },
    /// Resolve a ROM's artwork, fetching from the server if needed
    Art {
        term: String,
    },
    /// Show how a file hashes, whole and as archive members
    Hashcheck {
        file: PathBuf,
    },
    /// Download the BIOS set from the server into the library folder
    SyncBios,
    /// Sync save files and save states with the server
    SyncSaves {
        /// Report what would happen without writing or uploading anything
        #[arg(long)]
        dry_run: bool,
        /// Answer every conflict the same way instead of leaving them alone.
        /// The copy not kept is backed up to library/saves-backup/ first.
        #[arg(long, value_name = "local|server")]
        keep: Option<String>,
    },
}

/// Sync saves with the server.
///
/// `--dry-run` stops after the scan and prints what would be offered, which is
/// the safe way to see whether the ROM matching is right before anything
/// overwrites a save file.
async fn cmd_sync_saves(dry_run: bool, keep: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    let store = cache::Cache::open(Path::new(CACHE_DB))?;
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
    let ra = RetroArch::locate_in(&cfg.retroarch.ordered_paths())?;

    if dry_run {
        let candidates = moose_rack::savesync::scan(&store, &map, &ra.root)?;
        let (offered, skipped) = moose_rack::savesync::client_states(&candidates);
        println!("{} save(s) would be offered, {skipped} skipped", offered.len());
        for s in &offered {
            println!(
                "  rom {:>6}  {:<40} {:<10} {}",
                s.rom_id,
                s.file_name,
                s.slot.as_deref().unwrap_or("-"),
                s.emulator.as_deref().unwrap_or("-")
            );
        }

        // Save states go to a different endpoint and are compared differently,
        // so they get their own line rather than being folded into the count.
        let states: Vec<_> = candidates
            .iter()
            .filter(|c| c.kind == saves::Kind::State && c.canonical)
            .filter(|c| matches!(c.resolution, saves::Resolution::Resolved { .. }))
            .collect();
        println!("\n{} save state(s) would be compared against /api/states", states.len());
        for c in &states {
            let saves::Resolution::Resolved { rom_id, .. } = &c.resolution else { continue };
            println!(
                "  rom {:>6}  {:<40} {:<10} {}",
                rom_id,
                c.path.file_name().unwrap_or_default().to_string_lossy(),
                c.slot,
                c.core.as_deref().unwrap_or(&c.core_dir)
            );
        }
        return Ok(());
    }

    let client = cfg.server.client()?;
    let candidates = moose_rack::savesync::scan(&store, &map, &ra.root)?;
    let summary =
        moose_rack::savesync::run_all(
            &client,
            &candidates,
            &ra.root,
            Path::new("."),
            Path::new(&cfg.library.local_root),
        )
        .await?;
    for note in &summary.notes {
        println!("{note}");
    }
    println!("{}", summary.headline());

    if summary.conflicts.is_empty() {
        return Ok(());
    }

    // Without --keep there is nothing more this command can do: a conflict is a
    // question, and the CLI has no way to ask it mid-run. Print it and say how
    // to answer, rather than repeating the same stalemate on every run.
    let Some(keep) = keep else {
        println!("\n{} conflict(s) — nothing was written:", summary.conflicts.len());
        for c in &summary.conflicts {
            println!(
                "  {:<44} this machine {}   server {}",
                c.file_name,
                c.local_updated.as_deref().unwrap_or("?"),
                c.server_updated.as_deref().unwrap_or("?")
            );
        }
        println!(
            "\nAnswer them with `sync-saves --keep local` or `--keep server`.\n\
             The copy you do not keep is backed up to library/saves-backup/ first."
        );
        return Ok(());
    };

    let choice = match keep {
        "local" | "mine" => moose_rack::savesync::Keep::Local,
        "server" | "remote" => moose_rack::savesync::Keep::Server,
        other => bail!("--keep takes `local` or `server`, not {other:?}"),
    };
    println!("\nresolving {} conflict(s), keeping {keep}:", summary.conflicts.len());
    for c in &summary.conflicts {
        match moose_rack::savesync::resolve(
            &client,
            c,
            choice,
            &ra.root,
            Path::new(&cfg.library.local_root),
            Path::new("."),
        )
        .await
        {
            Ok(msg) => println!("  {msg}"),
            Err(e) => println!("  {}: FAILED — {e}", c.file_name),
        }
    }
    Ok(())
}

/// Fetch the BIOS set. Neo Geo and friends will not start without it, and the
/// failure in the emulator names neither the file nor where it should go.
async fn cmd_sync_bios() -> Result<()> {
    let cfg = Config::load()?;
    let client = cfg.server.client()?;
    let root = Path::new(&cfg.library.local_root);

    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let summary = moose_rack::bios::sync(&client, root, |done, total, name| {
        if interactive {
            print!("\r  {done}/{total}  {:<38}", name.chars().take(36).collect::<String>());
            use std::io::Write as _;
            std::io::stdout().flush().ok();
        }
    })
    .await?;

    if interactive {
        println!();
    }
    for note in summary.notes.iter().take(10) {
        println!("  {note}");
    }
    println!("{}", summary.headline());
    println!("into {}", moose_rack::bios::system_dir(root).display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Check => cmd_check().await,
        Command::Sync { full } => cmd_sync(full).await,
        Command::Browse => cmd_browse(),
        Command::Collections { group } => cmd_collections(group.as_deref()),
        Command::ScanEsde { root, roms, media } => {
            cmd_scan_esde(root.as_deref(), roms.as_deref(), media.as_deref())
        }
        Command::Probe { term, platform, sample, cores, frames, i_know_it_opens_windows } => {
            if !i_know_it_opens_windows {
                bail!(
                    "probe opens one RetroArch window per game per core, and macOS has no\n\
                     headless mode for it. Re-run with --i-know-it-opens-windows if that is\n\
                     what you want."
                );
            }
            cmd_probe(term.as_deref(), platform.as_deref(), sample, cores.as_deref(), frames).await
        }
        Command::Get { term, platform, collections, jobs } => {
            match (term, platform, collections) {
                (_, _, Some(grp)) => cmd_get_group(&grp, jobs).await,
                (_, Some(slug), None) => cmd_get_platform(&slug, jobs).await,
                (Some(t), None, None) => cmd_get(&t).await,
                (None, None, None) => {
                    bail!("give a search term, --platform <slug>, or --collections <group>")
                }
            }
        }
        Command::Scrape { platform, game, limit, videos, dry_run } => {
            cmd_scrape(platform.as_deref(), game.as_deref(), limit, videos, dry_run).await
        }
        Command::Coverage { probe, sample } => cmd_coverage(probe, sample).await,
        Command::ProbeReport { file } => cmd_probe_report(&file),
        Command::HashParity => {
            let cfg = Config::load()?;
            let client =
                cfg.server.client()?;
            parity::run(&client).await
        }
        Command::Scan => cmd_scan(),
        Command::Themes {
            install,
            available,
            get,
            logos_only,
            update,
        } => match (available, get, update) {
            // An empty string means --available was given with no filter.
            (Some(filter), _, _) => {
                cmd_themes_available(Some(filter.as_str()).filter(|f| !f.is_empty())).await
            }
            (_, Some(name), _) => cmd_themes_get(&name, logos_only).await,
            (_, _, true) => cmd_themes_update().await,
            _ => cmd_themes(install),
        },
        Command::Doctor => cmd_doctor(),
        Command::Config { fix } => cmd_config(fix),
        Command::Shaders { platform } => cmd_shaders(platform.as_deref()),
        Command::InstallRetroarch { version } => cmd_install_retroarch(version.as_deref()).await,
        Command::Cores { install, update } => cmd_cores(install, update).await,
        Command::Suggest => cmd_suggest(),
        Command::Launch {
            rom,
            core,
            fullscreen,
            go,
            pad,
        } => cmd_launch(&rom, go, core.as_deref(), fullscreen, pad.as_deref()).await,
        Command::Art { term } => cmd_art(&term).await,
        Command::Hashcheck { file } => cmd_hashcheck(&file),
        Command::SyncBios => cmd_sync_bios().await,
        Command::SyncSaves { dry_run, keep } => cmd_sync_saves(dry_run, keep.as_deref()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Depth is the whole point: a ROM can sit directly under its platform, or
    /// two levels down inside a folder ROM. Taking the parent directory would
    /// report `Shenmue (USA)` as the platform.
    #[test]
    fn the_platform_is_the_directory_directly_under_roms() {
        let cases = [
            ("/lib/roms/snes/Chrono Trigger.sfc", Some("snes")),
            ("/lib/roms/dc/Shenmue (USA)/Shenmue (USA).m3u", Some("dc")),
            ("/lib/roms/psx/MultiDisk/FF7/disc1.chd", Some("psx")),
            // `roms` at the filesystem root still works.
            ("/roms/nes/Metroid.nes", Some("nes")),
        ];
        for (path, want) in cases {
            assert_eq!(platform_from_path(Path::new(path)).as_deref(), want, "for {path}");
        }
    }

    /// Outside a library tree there is nothing to infer, and guessing would
    /// launch the wrong core. The caller falls back to asking the index.
    #[test]
    fn a_path_outside_a_library_infers_nothing() {
        for path in ["/Users/frank/Downloads/Game.sfc", "Game.sfc", "/roms"] {
            assert_eq!(platform_from_path(Path::new(path)), None, "for {path}");
        }
    }

    /// Only a directory literally named `roms` anchors the search, so a game
    /// under `my-roms/` is not silently mistaken for a library.
    #[test]
    fn the_anchor_directory_must_be_named_exactly_roms() {
        assert_eq!(platform_from_path(Path::new("/lib/my-roms/snes/Game.sfc")), None);
    }
}
