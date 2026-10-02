# NSIS template

Pinned upstream: tauri-cli-v2.11.5, crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi.
Upstream SHA256: 20F4ECC730DEFB71F1342EAEAEC4021DF13BE3D843ABBA0EFFE88EA5835FA079.

One local change after SemverCompare: a newer version over an existing NSIS install enables upstream UpdateMode and skips the maintenance choices. Same-version reinstall and WiX migration retain upstream behavior. Downgrades are disabled in configuration. Upstream CheckIfAppIsRunning prompts before closing the running app; settings under LocalAppData/CodexBadge are outside the installation directory and are preserved.

When upgrading the Tauri CLI, diff this template against the matching upstream version and carry only this change forward. Do not copy generated .nsi output with machine-specific paths.

No installer has been run during this implementation; installation/upgrade acceptance requires an isolated Windows user or VM.
