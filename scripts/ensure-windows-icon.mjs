import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const target = resolve(process.cwd(), 'src-tauri', 'icons', 'icon.ico');
const encoded = 'AAABAAEAEBAAAAAAIAC3AAAAFgAAAIlQTkcNChoKAAAADUlIRFIAAAAQAAAAEAgGAAAAH/P/YQAAAH5JREFUeJxjZOXg/s9AAWBB5nQevEm0xnJ7dQiDlYP7PysH9/++k0/+w9jEYJh6RlYO7v+dB28iTCQBdB68ycBEsi40MPAGsGATvPXuC1bFakI8xBmATSFFLsBnIMUuoE0sYEvSuJI5Vi+U26tjaMCZUqmSFwg5E5crGRgYGADP60ilvTJKgQAAAABJRU5ErkJggg==';
const bytes = Buffer.from(encoded, 'base64');

if (bytes.length < 128 || bytes.subarray(0, 4).toString('hex') !== '00000100') {
  throw new Error('Embedded Raphael Windows icon is invalid.');
}

mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, bytes);
console.log(`Prepared Windows icon: ${target} (${bytes.length} bytes)`);
