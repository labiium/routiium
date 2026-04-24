#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const { binaryName, detectPlatform } = require('./platform');

function candidateBinaries() {
  const platformInfo = detectPlatform();
  const nativeName = binaryName(platformInfo);
  const root = path.resolve(__dirname, '..');
  const candidates = [];

  if (process.env.ROUTIIUM_BINARY) candidates.push(process.env.ROUTIIUM_BINARY);
  candidates.push(path.join(__dirname, 'native', nativeName));
  candidates.push(path.join(root, 'target', 'release', nativeName));
  candidates.push(path.join(root, 'target', 'debug', nativeName));
  return candidates;
}

function findBinary() {
  for (const candidate of candidateBinaries()) {
    if (candidate && fs.existsSync(candidate)) return candidate;
  }
  return null;
}

const binary = findBinary();
if (!binary) {
  console.error('Routiium native binary was not found.');
  console.error('Try reinstalling with scripts enabled: npm install -g routiium');
  console.error('If you build Routiium yourself, set ROUTIIUM_BINARY=/path/to/routiium.');
  process.exit(127);
}

const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
child.on('error', (error) => {
  console.error(`Failed to start Routiium binary at ${binary}: ${error.message}`);
  process.exit(error.code === 'ENOENT' ? 127 : 1);
});
child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code == null ? 1 : code);
});
