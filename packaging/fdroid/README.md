# Publishing the phone application

The repository is built with `fdroidserver` and served as static files. There is no server
software to run and nothing to keep patched.

## Once

1. Make a signing keystore and **back it up off this machine**. Everything in
   `config.yml` explains why in more detail than it looks like it deserves: losing it
   re-identifies every phone in the house.

   ```sh
   keytool -genkey -v -keystore photo-sync.keystore -alias photo-sync \
       -keyalg RSA -keysize 4096 -validity 10000
   ```

2. Point `repo_url` and `archive_url` in `config.yml` at wherever the files will be served
   from. They have to be reachable from the phones, and that is the only requirement.

## Each release

```sh
cd android && ./gradlew :app:assembleRelease
cp app/build/outputs/apk/release/app-release.apk ../packaging/fdroid/repo/
cd ../packaging/fdroid && fdroid update --create-metadata
```

Then copy `repo/` and `archive/` to wherever they are served. A phone that has the repository
added picks the new version up on its own.

## The first phone

F-Droid needs the repository's fingerprint to trust it, which `fdroid update` prints. Adding
the repository by QR code carries the fingerprint with it and is easier to get right than
typing one.
