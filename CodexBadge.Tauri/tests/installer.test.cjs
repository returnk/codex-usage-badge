const fs=require('node:fs'), assert=require('node:assert/strict'), test=require('node:test');
test('installer skips maintenance choice for upgrades and retains same-version reinstall',()=>{
 const source=fs.existsSync('src-tauri/installer/installer.nsi') ? fs.readFileSync('src-tauri/installer/installer.nsi','utf8') : fs.readFileSync('artifacts/nsis-upstream-2.11.5.nsi','utf8');
 assert.match(source,/\$R0 = 1[\s\S]*?\$WixMode != 1[\s\S]*?StrCpy \$UpdateMode 1[\s\S]*?Abort/);
 assert.match(source,/\$R0 = 0[\s\S]*?addOrReinstall/);
 assert.match(source,/CheckIfAppIsRunning/);
 assert.ok(!source.includes('CodexBadge/tauri-settings.json'));
});
