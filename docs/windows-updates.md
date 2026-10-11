# Windows updates for the Bilalarham fork

The Windows desktop app checks the public GitHub Releases page for `Bilalarham/gridcraft` when it
starts. The check runs in the background; it stays quiet if there is no newer release or GitHub
cannot be reached. **Automate → About GridCraft → Check for updates** runs the same check and
shows the result.

When a newer Windows x64 release exists, GridCraft asks before downloading its `.msi` installer.
The download is checked against GitHub's SHA-256 asset digest when GitHub provides one. The app
then opens Windows Installer and waits for it to finish. Save open workbooks first, follow the
installer window and confirm the Windows permission prompt. GridCraft reports whether the update
completed or shows the Windows Installer exit code and keeps the downloaded installer if it fails.
This is an assisted update, not a silent install.

## Publish an update

1. Change `[workspace.package].version` in the root `Cargo.toml` and commit the change.
2. Push that commit to the fork.
3. Create and push a tag with the same version prefixed by `v`, for example `v0.5.3` when the
   workspace version is `0.5.3`.
4. GitHub Actions builds the Windows x64 MSI and portable ZIP, then publishes a GitHub Release.

The first release is also required before the updater can find an installer. An existing installation
must first be replaced with a build that contains this updater. Windows may ask for administrator
permission because the MSI installs for all users of the computer.
