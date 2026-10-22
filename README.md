# Raphael Story Generator

Raphael Story Generator is a local-first Tauri + React application for building persistent anime/manga stories and feeding them into a visual production pipeline.

## Current pipeline

```
User prompt
  -> Private web research
  -> Source-backed fact extraction
  -> Story analyzer / Story Bible
  -> Introduction + Chapter 1
  -> Chapter N continuation + optional user directive
  -> Scene extractor
  -> Image-builder prompt generation
  -> ComfyUI API workflow queue
```

## Canonical state

The app persists each story under the Tauri application data directory as JSON. The Story Bible stores:

- story metadata, genre, tags, demographic and content rating
- premise, conflict, themes and world rules
- characters with personality, appearance and clothing
- character relationships
- open plot threads and continuity notes
- chapters and extracted scenes

Chapter continuation is stateful. The optional user directive is passed only to the next chapter planner/writer; established canon remains authoritative and the resulting events are then written back into the story state.

## Private web research

Story creation performs real web research before the Story Architect stage when **web research is enabled**. Raphael does not use a hosted search API or a public SearXNG instance. The desktop app only accepts a local SearXNG endpoint and, by default, requires a local SOCKS5/Tor proxy for all external search/source traffic. Registry model thumbnails and metadata are read from the Raphael Model Registry API; Story Generator does not read the Model Manager SQLite database.

The repository includes a local privacy gateway under `privacy-search/`:

~~~text
Raphael -> localhost SearXNG -> Tor -> Internet search engines
        \-> local Tor proxy -> source pages
~~~

SearXNG is configured to route its outbound engine requests through Tor. Raphael also fetches each selected source page through the local Tor proxy, extracts readable text locally, and sends only the retrieved source context to the configured LLM for citation-backed fact extraction. Research facts and source URLs are persisted with the story.

The desktop application starts the bundled gateway automatically when it opens if the local SearXNG/Tor endpoints are not already healthy and web research is enabled. It installs the gateway files into the app's local data directory, generates a local SearXNG secret, starts Docker Compose, and waits for SearXNG and Tor routing to become ready. If Docker is unavailable during launch, Raphael retries startup every 15 seconds until the gateway starts or web research is disabled.

For manual operation or troubleshooting, the same stack can be started from `privacy-search/`:

~~~powershell
docker compose up -d --build
~~~

The default endpoints are `http://127.0.0.1:8080` for SearXNG and `socks5h://127.0.0.1:9050` for Tor. Public SearXNG URLs are rejected by the application, and private research fails closed when the required local proxy is unavailable.

The web-research extractor requires every factual claim to include source IDs and evidence. The Story Architect receives those source-backed facts and source URLs rather than being told to rely on its pretrained knowledge for external facts.

Privacy boundary: the SearXNG and Tor host ports are bound to `127.0.0.1`, and the app's default Registry/Ollama endpoints use loopback. However, user-configured LLM, ComfyUI, or Registry URLs can point to other machines or remote providers; those endpoints are not automatically local. A remote LLM receives the user prompt and extracted research. For end-to-end local privacy, use local Ollama and ComfyUI, and the bundled private research gateway. The optional LAN web host is bound to one detected private IPv4 address and serves the Story Generator UI plus its host-side API through a single port. It has no authentication or transport encryption, so use it only on a trusted LAN and never forward the port to the internet. The host's Ollama API stays on loopback; Raphael does not publish a separate Ollama endpoint.

## LLM configuration

The engine uses an OpenAI-compatible `/chat/completions` endpoint so it can point at local or remote providers. Web research is a separate network path: the search query and fetched web content go through the local privacy gateway first. The default configuration is intended for a local Ollama-style endpoint:

- Base URL: `http://127.0.0.1:11434/v1`
- Model: `qwen3:8b`
- API key: optional

Every LLM generation requests streaming when supported by the provider. The Tauri backend emits token/delta events to the frontend, where the Live Generation Trace shows the exact system prompt, exact user prompt, model, live response, completion state and errors for every generation stage. A dedicated Live Research Trace on the right also shows the active search, gateway, page-fetch and evidence-validation stages while a story is still being created. The Settings research smoke test checks Tor egress, executes a real SearXNG search, fetches sources and runs the research extractor; it does not just check whether a port is open.

The Engine Settings panel exposes a separate editable system prompt for each stage:

- Story Architect
- Continuity Writer
- Scene Director
- LoRA Selector
- Image Prompt Generator
- Web Research Extractor

Web research settings include the local SearXNG URL, local Tor/SOCKS5 proxy, fail-closed proxy requirement, result/source limits, and research extractor prompt. These settings are persisted with the local application settings.


### One-click LAN web hosting and host model selection

The desktop app has a single **WEB HOST** toggle in the top bar. Start hosting, then open the displayed address (for example, `http://<HOST-LAN-IP>:1425`) on another device connected to the same trusted LAN. Use **COPY ADDRESS** to share the link. The host serves the built Story Generator UI and handles API requests on the host machine, so stories, Registry access, ComfyUI calls, research and LLM inference use the host's configured services.

On a LAN browser, open **Settings → Ollama Auto-Detection → LOAD HOST MODELS**. Raphael queries the host machine's local Ollama and fills the model dropdown with the installed model IDs. Choosing a model and saving updates the host-side settings. The Ollama service remains on `127.0.0.1:11434`; the only LAN listener created by this feature is the Story Generator web app on port `1425`. There is no separate Ollama endpoint and no API key/Bearer token is needed to open the LAN page.

LAN hosting is unauthenticated plain HTTP. Anyone who can reach the LAN address can use the app and its host-side services, view host stories and initiate work with the host's configured models. Keep it on a trusted network, don't configure router port forwarding, and stop hosting when finished. The host PC's Windows Firewall may ask whether to allow the selected app port.
### Live service health

The **Raphael Services** panel probes the Registry, SearXNG local endpoint, ComfyUI `/system_stats`, and Ollama `/api/version` every 3 seconds while the app is open. Ollama checks the local `11434` endpoint by default, honors `OLLAMA_HOST` and `RAPHAEL_OLLAMA_URL`, and also considers a configured local LLM URL. Health requests have bounded timeouts; ComfyUI generation keeps its separate longer timeout. Status reflects API responses rather than a bare open TCP port.

## LAN Model Registry

By default, the Story Generator starts a local Model Registry on `127.0.0.1:43217`. A second PC on the same LAN must instead connect to the Registry running on the host PC.

On the **host PC**, start the Registry with a LAN bind and an explicit shared token:

~~~powershell
$env:RAPHAEL_REGISTRY_BIND = "0.0.0.0"
$env:RAPHAEL_REGISTRY_ALLOW_INSECURE_LAN = "true"
$env:RAPHAEL_REGISTRY_AUTH_TOKEN = "<strong-random-shared-token>"
raphael-registry server
~~~

Allow TCP port `43217` through the host Windows Firewall for the trusted LAN. On the **LAN PC**, configure Story Generator to use the host address and the same token before launching it:

~~~powershell
$env:RAPHAEL_REGISTRY_URL = "http://<HOST-LAN-IP>:43217"
$env:RAPHAEL_REGISTRY_AUTH_TOKEN = "<strong-random-shared-token>"
npm run tauri:dev
~~~

When `RAPHAEL_REGISTRY_URL` is set, Story Generator uses that existing Registry and will **not** start a second local instance, matching Raphael Image Generator behavior. The URL and `RAPHAEL_REGISTRY_AUTH_TOKEN` are honored by packaged builds too; `RAPHAEL_REGISTRY_TOKEN_FILE` can override the local token-file path. Without an explicit URL, the app probes `http://127.0.0.1:43217` and starts a local Registry when its executable is available. The Registry uses bearer-token authentication; plain HTTP LAN mode should only be used on a trusted network, or replaced with a TLS/reverse-proxy setup for an untrusted network.

## Prerequisites

For the full local production pipeline, Raphael currently expects:

- Raphael Model Registry running locally. The Story Generator starts it automatically when its executable or development source tree is available.
- Docker Desktop with Docker Compose for the bundled SearXNG + Tor private research gateway.
- An OpenAI-compatible LLM endpoint; Ollama on localhost is the default.
- ComfyUI for image generation. Raphael queues workflows, follows live execution progress, retrieves completed image outputs, and displays the result in the scene.

Web research can be disabled when Docker/Tor is not desired. Image generation can remain disabled until ComfyUI is configured.

## Development

Install dependencies:

```bash
npm install
```

Run the desktop application:

```bash
npm run tauri:dev
```

`npm run dev` starts the Vite frontend server used internally by Tauri during development; the application requires the Tauri desktop runtime.

## Notes

Scene extraction and image prompt generation remain separate from story canon. Image prompts query the shared Raphael Model Registry for compatible character LoRAs and pass only validated, matching character assets into the ComfyUI workflow builder.


## ComfyUI workflow queue

The image builder queues a scene through ComfyUI's `/prompt` API, then monitors the job through ComfyUI WebSocket events with an HTTP history fallback. Live sampling/node progress is shown directly on the scene card. When the workflow completes, Raphael downloads the first generated image output into app data and displays it in the scene.

Leave **API workflow template** blank to use the built-in workflow mirrored from Raphael Image Generator. It builds the graph with `UNETLoader` for Registry model files under `models/unet` or `models/diffusion_models`, otherwise `CheckpointLoaderSimple`; uses the same Anima `CLIPLoader` and `VAELoader`; adds `LoraLoaderModelOnly` nodes in style-first, character-second order; and finishes with `EmptyLatentImage`, `KSampler`, `VAEDecode`, and `SaveImage`. Defaults are 28 steps, CFG 6.5, and Euler sampling. As with Raphael Image Generator, this built-in graph assumes the same Anima CLIP/VAE files exist in ComfyUI. For a different model architecture, set a custom workflow template instead.

For a **custom workflow override**, paste a ComfyUI API-format workflow JSON template and use these placeholders inside string values:

- `{{POSITIVE_PROMPT}}`
- `{{NEGATIVE_PROMPT}}`
- `{{SEED}}`
- `{{STORY_ID}}`
- `{{SCENE_ID}}`
- `{{IMAGE_WIDTH}}`
- `{{IMAGE_HEIGHT}}`
- `{{IMAGE_SIZE}}`

The fixed image size is controlled in **Settings → ComfyUI → Fixed Image Size**, with a default of **1920×1080**. Width and height are applied to every scene; the image-prompt LLM cannot override the configured resolution. Each dimension must be between 512 and 4096 and divisible by 8, so landscape 1920×1080 and values such as 864 are accepted. Raphael injects the configured values into an Empty*Latent node or explicit size placeholders in custom workflows; a custom workflow without a valid size injection point is rejected.

Use `"{{SEED}}"` as the complete value of a numeric seed field. Raphael converts that exact placeholder into a JSON number; embedded text such as `"seed={{SEED}}"` remains a string.

### Dynamic LoRA workflow builder

For a custom workflow, the template must contain a `CheckpointLoaderSimple` or `CheckpointLoader` node. Raphael inserts one ComfyUI `LoraLoader` node per item and chains MODEL + CLIP through the stack. The built-in Image Generator graph instead uses `LoraLoaderModelOnly` nodes chained through MODEL only. Both modes build the LoRA chain deterministically with no fixed number of loader slots.

The dynamic stack is ordered: locked style LoRAs first, then only the character LoRAs matched to canonical characters in the current scene.

The stack is ordered: locked style LoRAs first, then scene-matched character LoRAs. Automatic selection is restricted to Registry entries tagged `character`, compatible with the selected checkpoint, and matched to a canonical scene character by Registry name, tags, or description. The selector never automatically adds generic style, concept, pose, setting, or background LoRAs. If no matching character LoRA exists, generation continues using the Story Bible's canonical appearance and clothing lock without a character LoRA. The selector chooses the LoRAs; the workflow builder only constructs the graph.

Automatic model discovery is handled through the shared Raphael Model Registry, the same metadata source used by Raphael Image Generator. When web research is enabled and the story prompt names an external character or series, the private research stage runs before Story Architect; per-scene character LoRA discovery runs when scene prompts are built.


## Local settings and secrets

The optional LLM API key is stored in the operating system credential store (Windows Credential Manager) and is never written to the application's JSON settings file. Raphael keeps only non-secret configuration in local settings. Do not use a shared Windows account for credentials you need to keep private.


## Image generation lifecycle

~~~text
Queue workflow
  -> queued
  -> ComfyUI WebSocket progress events
  -> current node + sampling step progress
  -> execution success/error
  -> /history/{prompt_id}
  -> /view image download
  -> persisted app-data image
  -> scene image preview
~~~

If the ComfyUI WebSocket is unavailable, Raphael falls back to /history/{prompt_id} and /queue polling so completed generations are still detected. Queued jobs are resumed when the application starts again.


The latest CI pass validates both frontend and Windows Rust packaging after each PR synchronization.

## Research visibility

When private research is enabled, the story view shows the actual search queries, fetched source titles/URLs/snippets, source-backed facts and the raw WEB RESEARCH LLM extractor response in the live trace. This makes it possible to inspect what search returned before the Story Architect uses the extracted facts.


SearXNG remains health-checked even when research is disabled, so the service panel reflects the API's actual availability rather than the feature toggle.


Feature-branch CI runs use the same concurrency group as pull-request runs, so the latest verification is the authoritative run instead of accumulating duplicate builds.


All actionable UI controls are rendered as semantic buttons, with runtime errors exposed through visible alert/error surfaces.


### Runtime service health
Raphael continuously probes the live Model Registry, SearXNG local endpoint, ComfyUI `/system_stats`, and Ollama `/api/version` endpoint on a 3-second UI refresh cadence. Ollama uses `http://127.0.0.1:11434` by default and can be pointed at a different local endpoint with `RAPHAEL_OLLAMA_URL`; `OLLAMA_HOST` is also recognized. Probes are API-level checks with bounded timeouts rather than static labels or bare TCP-connect checks.
