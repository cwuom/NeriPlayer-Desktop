import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createCipheriv, createDecipheriv, pbkdf2Sync } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const temporaryRoot = path.join(project, 'docs', 'tmp')
mkdirSync(temporaryRoot, { recursive: true })
const directory = mkdtempSync(path.join(temporaryRoot, 'neriplayer-config-codec-'))
const codec = path.join(project, 'src-tauri/src/config_backup.rs').replaceAll('\\', '/')
const aad = 'moe.ouom.neriplayer.config.encrypted|1|PBKDF2-HMAC-SHA256|600000|AES-256-GCM'
const password = ' 密码🔐 e\u0301 '
const plain = { kind: 'moe.ouom.neriplayer.config', formatVersion: 1, settings: { strings: { audio_quality: 'lossless' } } }
const salt = Buffer.from(Array.from({ length: 16 }, (_, i) => i))
const iv = Buffer.from(Array.from({ length: 12 }, (_, i) => i + 16))
const key = pbkdf2Sync(password, salt, 600000, 32, 'sha256')
const cipher = createCipheriv('aes-256-gcm', key, iv)
cipher.setAAD(Buffer.from(aad))
const ciphertext = Buffer.concat([cipher.update(JSON.stringify(plain)), cipher.final(), cipher.getAuthTag()])
const fixture = { kind: 'moe.ouom.neriplayer.config.encrypted', formatVersion: 1, kdf: 'PBKDF2-HMAC-SHA256', iterations: 600000, cipher: 'AES-256-GCM', salt: salt.toString('base64'), iv: iv.toString('base64'), ciphertext: ciphertext.toString('base64') }

try {
  writeFileSync(path.join(directory, 'rust-toolchain.toml'), readFileSync(path.join(project, 'rust-toolchain.toml')))
  writeFileSync(path.join(directory, 'Cargo.toml'), `[package]\nname = "neriplayer-config-codec-test"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\naes-gcm = "0.10"\npbkdf2 = "0.12"\nsha2 = "0.10"\nrand = "0.8"\nbase64 = "0.22"\nserde = { version = "1", features = ["derive"] }\nserde_json = "1"\n`)
  writeFileSync(path.join(directory, 'fixture.json'), JSON.stringify(fixture))
  writeFileSync(path.join(directory, 'main.rs'), `#[path = ${JSON.stringify(codec)}]\nmod config_backup;\nfn main() {\n let input = std::fs::read("fixture.json").unwrap();\n let password = ${JSON.stringify(password)};\n let plain = config_backup::decode(&input, Some(password)).unwrap();\n std::fs::write("decoded.json", serde_json::to_vec(&plain).unwrap()).unwrap();\n std::fs::write("encoded.json", config_backup::encode(&plain, Some(password)).unwrap()).unwrap();\n}\n`)
  const manifest = readFileSync(path.join(directory, 'Cargo.toml'), 'utf8')
  writeFileSync(path.join(directory, 'Cargo.toml'), `${manifest}\n[[bin]]\nname = "codec"\npath = "main.rs"\n`)
  execFileSync('cargo', ['test', '--quiet'], { cwd: directory, stdio: 'inherit', env: process.env })
  execFileSync('cargo', ['run', '--quiet'], { cwd: directory, stdio: 'inherit', env: process.env })
  assert.deepEqual(JSON.parse(readFileSync(path.join(directory, 'decoded.json'), 'utf8')), plain)
  const exported = JSON.parse(readFileSync(path.join(directory, 'encoded.json'), 'utf8'))
  assert.equal(exported.iterations, 600000)
  const exportedKey = pbkdf2Sync(password, Buffer.from(exported.salt, 'base64'), 600000, 32, 'sha256')
  const packed = Buffer.from(exported.ciphertext, 'base64')
  const decipher = createDecipheriv('aes-256-gcm', exportedKey, Buffer.from(exported.iv, 'base64'))
  decipher.setAAD(Buffer.from(aad))
  decipher.setAuthTag(packed.subarray(-16))
  assert.deepEqual(JSON.parse(Buffer.concat([decipher.update(packed.subarray(0, -16)), decipher.final()])), plain)
  console.log('PASS: Rust codec tests and Node crypto bidirectional interoperability')
} finally {
  assert.equal(path.dirname(directory), temporaryRoot)
  assert.ok(path.basename(directory).startsWith('neriplayer-config-codec-'))
  rmSync(directory, { recursive: true, force: true })
}
