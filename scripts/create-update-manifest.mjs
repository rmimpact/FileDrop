import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const [version, releasePath, signaturesDirectory, outputPath] = process.argv.slice(2);

if (!version || !releasePath || !signaturesDirectory || !outputPath) {
  console.error('Usage: node scripts/create-update-manifest.mjs <version> <release.json> <signatures-dir> <output.json>');
  process.exit(1);
}

const release = JSON.parse(await readFile(releasePath, 'utf8'));
const signatureNames = await readdir(signaturesDirectory);

async function updaterFor(signatureSuffix) {
  const signatureName = signatureNames.find((name) => name.endsWith(signatureSuffix));
  if (!signatureName) {
    throw new Error(`Missing updater signature ending in ${signatureSuffix}`);
  }

  const assetName = signatureName.slice(0, -'.sig'.length);
  const asset = release.assets.find((candidate) => candidate.name === assetName);
  if (!asset?.browser_download_url) {
    throw new Error(`Missing release asset ${assetName}`);
  }

  return {
    signature: (await readFile(join(signaturesDirectory, signatureName), 'utf8')).trim(),
    url: asset.browser_download_url,
  };
}

const macOS = await updaterFor('.app.tar.gz.sig');
const windows = await updaterFor('-setup.exe.sig');

const manifest = {
  version,
  notes: release.body ?? '',
  pub_date: new Date().toISOString(),
  platforms: {
    'darwin-aarch64': macOS,
    'darwin-x86_64': macOS,
    'darwin-universal': macOS,
    'windows-x86_64': windows,
    'windows-x86_64-nsis': windows,
  },
};

await writeFile(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
