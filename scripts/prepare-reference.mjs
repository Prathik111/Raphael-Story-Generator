import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
const project = path.resolve(root, '..');
const source = path.join(project, 'reference-source', 'index.html');
const outDir = path.join(project, 'public', 'reference');

fs.mkdirSync(outDir, { recursive: true });

if (!fs.existsSync(source)) {
  throw new Error('Missing reference-source/index.html');
}

let html = fs.readFileSync(source, 'utf8');

html = html.replace(
  '<script src="https://unpkg.com/three@0.128.0/build/three.min.js"></script>',
  '<script src="./three.min.js"></script>'
);

const css = `
<style>
html,body,#canvas-wrap{width:100%!important;height:100%!important;margin:0!important;overflow:hidden!important}
body{background:#030b08!important;pointer-events:none!important;filter:none!important;transform:none!important}
body>*:not(#canvas-wrap){display:none!important}
#canvas-wrap{position:fixed!important;inset:0!important;filter:none!important;transform:none!important}
#canvas-wrap.idle-blur{filter:none!important;transform:none!important}
canvas{width:100%!important;height:100%!important;display:block!important}
</style>
<script>
window.addEventListener('DOMContentLoaded',()=>{document.body.classList.remove('startup-intro','startup-active','lens-blur','mini','panel-window');});
</script>
`;

html = html.replace('</head>', css + '</head>');
fs.writeFileSync(path.join(outDir, 'index.html'), html);

const threeSource = path.join(project, 'node_modules', 'three', 'build', 'three.min.js');
if (fs.existsSync(threeSource)) {
  fs.copyFileSync(threeSource, path.join(outDir, 'three.min.js'));
}

console.log('Prepared Raphael background');
