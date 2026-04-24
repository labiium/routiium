#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');

const root = path.resolve(__dirname, '..');
const pkg = require(path.join(root, 'package.json'));
const cargoToml = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
const cargoVersion = cargoToml.match(/^version = "([^"]+)"/m)?.[1];
const errors = [];

if (pkg.name !== 'routiium') errors.push(`package.json name must be routiium, got ${pkg.name}`);
if (pkg.private) errors.push('package.json must not be private for npm publishing');
if (!pkg.bin || pkg.bin.routiium !== 'npm/routiium.js') errors.push('package.json bin.routiium must point to npm/routiium.js');
if (pkg.version !== cargoVersion) errors.push(`package.json version (${pkg.version}) must match Cargo.toml (${cargoVersion})`);
for (const file of ['npm/routiium.js', 'npm/postinstall.js', 'npm/platform.js', 'Cargo.toml', 'Cargo.lock', 'README.md', 'LICENSE']) {
  if (!fs.existsSync(path.join(root, file))) errors.push(`required publish file missing: ${file}`);
}
if (errors.length > 0) { console.error(errors.map((error) => `- ${error}`).join('\n')); process.exit(1); }
console.log('routiium npm publish checks passed');
