const fs=require('node:fs'), assert=require('node:assert/strict'), test=require('node:test');
test('installer skips maintenance choice for upgrades and retains same-version reinstall',()=>{
 const source=fs.existsSync('src-tauri/installer/installer.nsi') ? fs.readFileSync('src-tauri/installer/installer.nsi','utf8') : fs.readFileSync('artifacts/nsis-upstream-2.11.5.nsi','utf8');
 assert.match(source,/\$R0 = 1[\s\S]*?\$WixMode != 1[\s\S]*?StrCpy \$UpdateMode 1[\s\S]*?Abort/);
 assert.match(source,/\$R0 = 0[\s\S]*?addOrReinstall/);
 assert.match(source,/CheckIfAppIsRunning/);
 assert.ok(!source.includes('CodexBadge/tauri-settings.json'));
});
test('uninstall removes only its own startup entry and preserves updates',()=>{
 const source=fs.readFileSync('src-tauri/installer/installer.nsi','utf8');
 const cleanup=source.slice(source.indexOf('; Removes the Autostart entry'),source.indexOf('; Delete app data if'));
 assert.match(cleanup,/\$\{If\} \$UpdateMode <> 1/);
 assert.match(cleanup,/ReadRegStr \$\w+ HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "CodexBadgeTauri"/);
 assert.match(cleanup,/\$\{If\} \$\w+ == "\$\\"\$INSTDIR\\\$\{MAINBINARYNAME\}\.exe\$\\""/);
 assert.match(cleanup,/DeleteRegValue HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "CodexBadgeTauri"/);
});
test('selected uninstall data cleanup names only Tauri files in the shared directory',()=>{
 const source=fs.readFileSync('src-tauri/installer/installer.nsi','utf8');
 const cleanup=source.slice(source.indexOf('; Delete app data if'),source.indexOf('!ifmacrodef NSIS_HOOK_POSTUNINSTALL'));
 assert.match(cleanup,/\$\{If\} \$DeleteAppDataCheckboxState = 1\s+\$\{AndIf\} \$UpdateMode <> 1/);
 const owned=[...cleanup.matchAll(/Delete "\$LOCALAPPDATA\\CodexBadge\\([^"\r\n]+)"/g)].map(match=>match[1]).sort();
 assert.deepEqual(owned,['tauri-diagnostics.log','tauri-diagnostics.log.old','tauri-settings.json','tauri-settings.json.tmp','tauri-update-notice.json','tauri-update-notice.json.tmp']);
 assert.doesNotMatch(source,/RmDir[^\r\n]*[\\/]CodexBadge"|Delete[^\r\n]*[\\/]CodexBadge[\\/](?:\*|settings\.json)/i);
});
