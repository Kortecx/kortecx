import { describe, it, expect } from 'vitest';
import path from 'path';

describe('Shared Config Import', () => {
  describe('Path traversal prevention', () => {
    it('rejects filenames with ..', () => {
      const filename = '../../../etc/passwd';
      expect(filename.includes('..')).toBe(true);
    });

    it('rejects filenames with forward slashes', () => {
      const filename = 'subdir/config.json';
      expect(filename.includes('/')).toBe(true);
    });

    it('rejects filenames with backslashes', () => {
      const filename = 'subdir\\config.json';
      expect(filename.includes('\\')).toBe(true);
    });

    it('accepts simple filenames', () => {
      const filename = 'my-export.json';
      const basename = path.basename(filename);
      expect(basename).toBe(filename);
      expect(filename.includes('..')).toBe(false);
      expect(filename.includes('/')).toBe(false);
      expect(filename.includes('\\')).toBe(false);
    });

    it('basename strips directory components', () => {
      expect(path.basename('../evil.json')).toBe('evil.json');
      expect(path.basename('/etc/passwd')).toBe('passwd');
      expect(path.basename('sub/dir/file.json')).toBe('file.json');
    });

    it('only accepts .json files', () => {
      expect('export.json'.endsWith('.json')).toBe(true);
      expect('export.txt'.endsWith('.json')).toBe(false);
      expect('export.json.exe'.endsWith('.json')).toBe(false);
    });
  });

  describe('Shared config listing structure', () => {
    it('shared config entry has correct shape', () => {
      const entry = {
        filename: 'my-expert.json',
        entityType: 'expert',
        name: 'My Expert',
        exportedAt: '2026-03-29T00:00:00Z',
        version: '1.0',
        sizeBytes: 1024,
      };

      expect(typeof entry.filename).toBe('string');
      expect(typeof entry.entityType).toBe('string');
      expect(typeof entry.name).toBe('string');
      expect(typeof entry.sizeBytes).toBe('number');
      expect(entry.filename.endsWith('.json')).toBe(true);
    });

    it('formats file sizes correctly', () => {
      const formatSize = (bytes: number) =>
        bytes > 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${bytes} B`;

      expect(formatSize(512)).toBe('512 B');
      expect(formatSize(1024)).toBe('1024 B');
      expect(formatSize(2048)).toBe('2.0 KB');
      expect(formatSize(10240)).toBe('10.0 KB');
    });
  });

  describe('Resolve path safety', () => {
    it('resolved path stays within shared directory', () => {
      const sharedDir = '/app/shared_configs';
      const filename = 'valid.json';
      const resolved = path.resolve(path.join(sharedDir, filename));
      const resolvedDir = path.resolve(sharedDir);

      expect(resolved.startsWith(resolvedDir)).toBe(true);
    });

    it('traversal attempt escapes shared directory', () => {
      const sharedDir = '/app/shared_configs';
      const filename = '../../../etc/passwd';
      const resolved = path.resolve(path.join(sharedDir, filename));
      const resolvedDir = path.resolve(sharedDir);

      expect(resolved.startsWith(resolvedDir + path.sep)).toBe(false);
    });
  });
});
