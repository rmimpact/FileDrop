import { readFile, writeFile } from 'node:fs/promises';

const nextVersion = process.argv[2];
const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

if (!nextVersion || !semverPattern.test(nextVersion)) {
  console.error('Usage: npm run release:version -- 1.2.3');
  process.exit(1);
}

async function updateJson(path, update) {
  const value = JSON.parse(await readFile(path, 'utf8'));
  update(value);
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

await updateJson('package.json', (value) => {
  value.version = nextVersion;
});

await updateJson('package-lock.json', (value) => {
  value.version = nextVersion;
  value.packages[''].version = nextVersion;
});

await updateJson('src-tauri/tauri.conf.json', (value) => {
  value.version = nextVersion;
});

const cargoPath = 'src-tauri/Cargo.toml';
const cargo = await readFile(cargoPath, 'utf8');
await writeFile(
  cargoPath,
  cargo.replace(
    /(\[package\][\s\S]*?\nversion = ")[^"]+("\n)/,
    `$1${nextVersion}$2`,
  ),
);

const cargoLockPath = 'src-tauri/Cargo.lock';
const cargoLock = await readFile(cargoLockPath, 'utf8');
await writeFile(
  cargoLockPath,
  cargoLock.replace(
    /(\[\[package\]\]\nname = "filedrop"\nversion = ")[^"]+("\n)/,
    `$1${nextVersion}$2`,
  ),
);

const componentPath = 'src/app/app.component.ts';
const component = await readFile(componentPath, 'utf8');
await writeFile(
  componentPath,
  component.replace(/appVersion = '[^']+';/, `appVersion = '${nextVersion}';`),
);

console.log(`FileDrop is ready to be released as version ${nextVersion}.`);
