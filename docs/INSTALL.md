# Installing it, and the first session

Two halves: a desktop application on a Linux computer and an application on each phone. They
find each other on the local network. There is no account and nothing leaves the house.

## The computer

Arch, from the package:

```sh
makepkg -si          # in packaging/
```

Anything else, from source:

```sh
cd desktop && cargo build --release --bin photo-sync
install -Dm755 target/release/photo-sync ~/.local/bin/photo-sync
install -Dm644 ../packaging/photo-sync.desktop ~/.local/share/applications/photo-sync.desktop
install -Dm644 ../packaging/photo-sync.svg \
    ~/.local/share/icons/hicolor/scalable/apps/photo-sync.svg
```

Start it once from the menu. It puts an item in the tray and opens a window. In **Settings**,
check where the vault is — `~/Pictures/Camera` unless changed — and turn on **Start when I log
in** so it is there when a phone looks for it.

Nothing runs as root, nothing is enabled as a service, and nothing listens on the network until
the application is running. Changing the vault path later moves nothing: the photographs stay
where they are, and moving them is a decision for a person and a file manager.

## The phones

Add the repository to F-Droid — by QR code, which carries the fingerprint that makes it
trustworthy — and install Photo Sync from it. `packaging/fdroid/README.md` covers setting the
repository up the first time.

Onboarding asks for four things in order. Only the first is required.

1. **Access to photographs.** Without it there is nothing to do.
2. **Media management.** Makes deleting silent after the one confirmation in the application.
   Declining it means Android asks again for each batch, which works and is tedious.
3. **Battery-optimisation exemption.** Without it, long transfers stop when the screen does.
4. **Brand-specific steps.** See the table below. These are the ones that catch people out.

## Pairing

Open the application on the phone with the computer's window open. The phone finds the
computer and both screens show the same six digits. Check they match, and confirm on both.

That is the only time anything is confirmed. From then on the two recognise each other by key.
If the computer is reinstalled its key changes, and the phone will say so rather than trust the
new one quietly — that is a re-pair, and it is deliberate.

A phone stores exactly one computer. Pairing with a second replaces the first.

## The first session

Open the application on the phone. It shows what it found — "1,180 new photos, 6.4 GB — 2,034
already safe" — and waits for **Start**. That is one tap.

When the transfer finishes, it offers to free the space and waits again. That is the second
tap, and it is the last: a session is two taps, and fewer when there is nothing to do.

A photograph is only ever deleted from the phone after the computer has proved it holds the
same bytes. If anything about a file has changed since it was backed up, the phone keeps it and
says so.

## Brands that need extra steps

Every phone below kills background applications more aggressively than Android requires. The
transfer survives the screen going off *only* if these are set. The onboarding flow shows the
same steps with pictures; this table is for looking up afterwards.

| Brand | System | What to set |
|---|---|---|
| Xiaomi, Redmi, POCO | MIUI, HyperOS | Settings → Apps → Photo Sync → **Autostart** on; **Battery saver** → No restrictions; in Recents, pull the card down and **lock** it |
| Oppo, realme | ColorOS | Settings → Battery → **Allow background running**; App management → Photo Sync → **Auto-launch** on; lock in Recents |
| OnePlus | OxygenOS | Settings → Battery → Battery optimisation → Photo Sync → **Don't optimise**; **Advanced optimisation** → Deep optimisation off |
| Vivo, iQOO | OriginOS, FuntouchOS | Settings → Battery → **High background power consumption** → allow Photo Sync; iManager → App manager → **Autostart** |
| Samsung | One UI | Settings → Battery → Background usage limits → **Never sleeping apps** → add Photo Sync |
| Others | Stock Android | The battery-optimisation exemption from onboarding is enough |

If a transfer keeps stopping when the screen goes off, one of these is missing. It is worth
checking them in the order above.

## When something is wrong

**The phone cannot find the computer.** They must be on the same network, and the computer's
application must be running — check the tray. Some routers block multicast between wireless
devices ("AP isolation" or "client isolation"); that setting stops discovery.

**The phone says the computer is not the one it knows.** The computer's key changed, which
means it was reinstalled — or something is pretending to be it. Re-pair only if the first
explains it.

**A photograph is never freed.** The phone keeps anything that has changed since it was backed
up. If the same photograph is kept session after session and never re-sent, the computer's
record of it is stale: the window offers **force re-import**, which drops that record so the
next session sends the file again.

**Logs.** The computer writes them to `~/.local/state/photo-sync/`, one file per day.
