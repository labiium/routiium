'use strict';

const os = require('os');

function detectPlatform() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === 'linux' && arch === 'x64') return { triple: 'x86_64-unknown-linux-gnu', exe: '' };
  if (platform === 'linux' && arch === 'arm64') return { triple: 'aarch64-unknown-linux-gnu', exe: '' };
  if (platform === 'darwin' && arch === 'x64') return { triple: 'x86_64-apple-darwin', exe: '' };
  if (platform === 'darwin' && arch === 'arm64') return { triple: 'aarch64-apple-darwin', exe: '' };
  if (platform === 'win32' && arch === 'x64') return { triple: 'x86_64-pc-windows-msvc', exe: '.exe' };

  return { triple: `${platform}-${arch}`, exe: platform === 'win32' ? '.exe' : '', unsupported: true };
}

function binaryName(platformInfo = detectPlatform()) {
  return `routiium${platformInfo.exe}`;
}

function assetName(version, platformInfo = detectPlatform()) {
  return `routiium-${platformInfo.triple}${platformInfo.exe}`;
}

function defaultDownloadUrl(version, platformInfo = detectPlatform()) {
  const base = process.env.ROUTIIUM_NPM_RELEASE_BASE_URL
    || `https://github.com/labiium/routiium/releases/download/v${version}`;
  return `${base.replace(/\/$/, '')}/${assetName(version, platformInfo)}`;
}

module.exports = { assetName, binaryName, defaultDownloadUrl, detectPlatform };
