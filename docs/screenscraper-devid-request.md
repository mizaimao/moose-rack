# ScreenScraper developer credentials — request draft

Post in the **WebAPI** section of the ScreenScraper forum:
<https://www.screenscraper.fr/forumsujets.php?frub=12>

API documentation, for reference: <https://www.screenscraper.fr/webapi2.php>

You need an ordinary screenscraper.fr account first; the credentials are issued
to the *software*, not to the person, which is why an account alone cannot call
the API.

Their forum is French-speaking, so a French version is below the English one.
Post whichever you prefer — or both, English underneath.

---

## English

**Subject:** devid request — moose-rack (open source RomM client)

> Hello,
>
> I would like to request API credentials for **moose-rack**, an open source
> desktop client for self-hosted [RomM](https://romm.app) game libraries. It runs
> on macOS, Windows and Linux, and launches games in RetroArch from a gamepad.
>
> Source: <https://github.com/mizaimao/moose-rack>
>
> I would use the API to scrape metadata and media for the user's own library —
> box art, cartridge, miximage, marquee, screenshots — stored in the ES-DE media
> layout so it stays interchangeable with ES-DE itself.
>
> Each installation is one user, signing in with their own ScreenScraper
> account, and the client is single-threaded with a delay between requests. It
> respects the thread allowance of the account it is using and does not
> parallelise beyond it.
>
> softname: `moose-rack`
>
> Thank you for the work you put into the database.

---

## Français

**Sujet :** demande de devid — moose-rack (client RomM open source)

> Bonjour,
>
> Je souhaite demander des identifiants API pour **moose-rack**, un client
> de bureau open source pour les bibliothèques de jeux auto-hébergées
> [RomM](https://romm.app). Il fonctionne sur macOS, Windows et Linux, et lance
> les jeux dans RetroArch à la manette.
>
> Code source : <https://github.com/mizaimao/moose-rack>
>
> J'utiliserais l'API pour récupérer les métadonnées et les médias de la
> bibliothèque personnelle de l'utilisateur — jaquette, support, miximage,
> marquee, captures d'écran — stockés selon l'arborescence média d'ES-DE afin de
> rester interchangeables avec ES-DE.
>
> Chaque installation correspond à un seul utilisateur, qui se connecte avec son
> propre compte ScreenScraper. Le client est mono-thread avec une pause entre
> les requêtes, et respecte le nombre de threads autorisé par le compte utilisé.
>
> softname : `moose-rack`
>
> Merci pour le travail accompli sur la base de données.

---

## What to do with the credentials

They go in `config.toml` under `[scraper]`:

```toml
ssid = "your screenscraper login"
sspassword = "your screenscraper password"
devid = "issued to you"
devpassword = "issued to you"
softname = "moose-rack"
max_threads = 1
```

`max_threads` is not decoration. ScreenScraper allocates simultaneous
connections by account tier and answers an exceeded allowance with a rejection
rather than a picture, so a client that ignores it scrapes nothing and looks
broken while doing it.

Until the credentials arrive, the app scrapes through the RomM server's own
ScreenScraper account instead — see `src/scrape.rs` for why that route is
legitimate and what it costs.
