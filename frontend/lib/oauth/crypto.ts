/**
 * Token Encryption — AES-256-GCM for OAuth token storage.
 *
 * OAUTH_ENCRYPTION_KEY must be a 64-char hex string (32 bytes).
 * Generate with: openssl rand -hex 32
 */

import { randomBytes, createCipheriv, createDecipheriv } from 'crypto';

const ALGORITHM = 'aes-256-gcm';
const IV_LENGTH = 16;
const AUTH_TAG_LENGTH = 16;

function getKey(): Buffer {
  const hex = process.env.OAUTH_ENCRYPTION_KEY;
  if (!hex || hex.length !== 64) {
    throw new Error(
      'OAUTH_ENCRYPTION_KEY is missing or invalid.\n' +
      '  Must be a 64-character hex string (32 bytes).\n' +
      '  Generate with: openssl rand -hex 32'
    );
  }
  return Buffer.from(hex, 'hex');
}

/** Encrypt a plaintext string → base64 ciphertext (iv + tag + encrypted). */
export function encryptToken(plaintext: string): string {
  const key = getKey();
  const iv = randomBytes(IV_LENGTH);
  const cipher = createCipheriv(ALGORITHM, key, iv);
  const encrypted = Buffer.concat([cipher.update(plaintext, 'utf8'), cipher.final()]);
  const tag = cipher.getAuthTag();
  // Format: iv (16) + tag (16) + ciphertext
  return Buffer.concat([iv, tag, encrypted]).toString('base64');
}

/** Decrypt a base64 ciphertext → plaintext string. */
export function decryptToken(ciphertext: string): string {
  const key = getKey();
  const buf = Buffer.from(ciphertext, 'base64');
  const iv = buf.subarray(0, IV_LENGTH);
  const tag = buf.subarray(IV_LENGTH, IV_LENGTH + AUTH_TAG_LENGTH);
  const encrypted = buf.subarray(IV_LENGTH + AUTH_TAG_LENGTH);
  const decipher = createDecipheriv(ALGORITHM, key, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(encrypted), decipher.final()]).toString('utf8');
}

/** Generate a random PKCE code verifier (43-128 chars, URL-safe). */
export function generateCodeVerifier(): string {
  return randomBytes(32).toString('base64url');
}

/** Generate a PKCE code challenge from a verifier (S256). */
export async function generateCodeChallenge(verifier: string): Promise<string> {
  const { createHash } = await import('crypto');
  return createHash('sha256').update(verifier).digest('base64url');
}

/** Generate a random state parameter for OAuth CSRF protection. */
export function generateState(): string {
  return randomBytes(16).toString('hex');
}
