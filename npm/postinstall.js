#!/usr/bin/env node
'use strict';

const fs = require('fs');
const https = require('https');
const path = require('path');
const { spawnSync } = require('child_process');
const { binaryName, defaultDownloadUrl, detectPlatform } = require('./platform');

const root = path.resolve(__dirname, '..');
const pkg = require(path.join(root, 'package.json'));
const platformInfo = detectPlatform();
const nativeDir = path.join(__dirname, 'native');
const binaryPath = path.join(nativeDir, binaryName(platformInfo));

function log(message) { console.log(`[routiium] ${message}`); }
function warn(message) { console.warn(`[routiium] ${message}`); }
function ensureNativeDir() { fs.mkdirSync(nativeDir, { recursive: true }); }
function chmodExecutable(file) { if (process.platform !== 'win32') fs.chmodSync(file, 0o755); }
function copyBinary(source) { ensureNativeDir(); fs.copyFileSync(source, binaryPath); chmodExecutable(binaryPath); }
function run(command, args, options = {}) {
  return spawnSync(command, args, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32', ...options });
}
function commandWorks(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'ignore', shell: process.platform === 'win32' });
  return result.status === 0;
}
function download(url, destination, redirects = 0) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, { headers: { 'User-Agent': `routiium-npm/${pkg.version}` } }, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode) && response.headers.location) {
        response.resume();
        if (redirects >= 5) return reject(new Error('too many redirects'));
        download(response.headers.location, destination, redirects + 1).then(resolve, reject);
        return;
      }
      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`HTTP ${response.statusCode}`));
        return;
      }
      ensureNativeDir();
      const tempPath = `${destination}.download`;
      const file = fs.createWriteStream(tempPath, { mode: 0o755 });
      response.pipe(file);
      file.on('finish', () => file.close(() => { fs.renameSync(tempPath, destination); chmodExecutable(destination); resolve(); }));
      file.on('error', (error) => { fs.rmSync(tempPath, { force: true }); reject(error); });
    });
    request.on('error', reject);
    request.setTimeout(30000, () => request.destroy(new Error('download timed out')));
  });
}

async function install() {
  if (fs.existsSync(binaryPath)) { log(`native binary already present at ${path.relative(root, binaryPath)}`); return; }
  if (process.env.ROUTIIUM_BINARY) { copyBinary(process.env.ROUTIIUM_BINARY); log(`copied native binary from ROUTIIUM_BINARY to ${path.relative(root, binaryPath)}`); return; }
  if (!platformInfo.unsupported && process.env.ROUTIIUM_NPM_SKIP_DOWNLOAD !== '1') {
    const url = process.env.ROUTIIUM_NPM_BINARY_URL || defaultDownloadUrl(pkg.version, platformInfo);
    try { log(`downloading ${url}`); await download(url, binaryPath); log(`installed native binary for ${platformInfo.triple}`); return; }
    catch (error) { warn(`prebuilt binary unavailable (${error.message}); falling back to local cargo build`); }
  } else if (platformInfo.unsupported) {
    warn(`no prebuilt binary target for ${platformInfo.triple}; falling back to local cargo build`);
  }
  if (process.env.ROUTIIUM_NPM_SKIP_BUILD === '1') throw new Error('ROUTIIUM_NPM_SKIP_BUILD=1 set and no prebuilt Routiium binary is installed');
  if (!commandWorks('cargo', ['--version'])) throw new Error('cargo is required to build Routiium from source when no prebuilt binary is available');
  log('building native binary with cargo build --release --locked');
  const result = run('cargo', ['build', '--release', '--locked']);
  if (result.status !== 0) throw new Error(`cargo build failed with exit code ${result.status}`);
  const built = path.join(root, 'target', 'release', binaryName(platformInfo));
  if (!fs.existsSync(built)) throw new Error(`cargo build completed but ${built} was not found`);
  copyBinary(built);
  log(`installed locally built binary at ${path.relative(root, binaryPath)}`);
}

install().catch((error) => {
  console.error(`[routiium] install failed: ${error.message}`);
  console.error('[routiium] See https://github.com/labiium/routiium/releases or install Rust and retry.');
  process.exit(1);
});
