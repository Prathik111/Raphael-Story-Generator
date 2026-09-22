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

Story creation performs real web research before the Story Architect stage when **web research is enabled**. Raphael does not use a hosted search API or a public SearXNG instance. The desktop app only accepts a local SearXNG endpoint and, by default, requires a local SOCKS5/Tor proxy for all external search/source traffic.

The repository includes a local privacy gateway under `privacy-search/`:

~~~text
Raphael -> localhost SearXNG -> Tor -> Internet search engines
        \-> local Tor proxy -> source pages
~~~

SearXNG is configured to route its outbound engine requests through Tor. Raphael also fetches each selected source page through the local Tor proxy, extracts readable text locally, and sends only the retrieved source context to the configured LLM for citation-backed fact extraction. Research facts and source URLs are persisted with the story.

The desktop application now starts the bundled gateway automatically when it opens if the local SearXNG/Tor endpoints are not already healthy. It installs the gateway files into the app's local data directory, generates a local SearXNG secret, starts Docker Compose, and waits for both SearXNG and Tor routing to become ready.

For manual operation or troubleshooting, the same stack can be started from `privacy-search/`:

~~~powershell
docker compose up -d --build
~~~

The default endpoints are `http://127.0.0.1:8080` for SearXNG and `socks5h://127.0.0.1:9050` for Tor. Public SearXNG URLs are rejected by the application, and private research fails closed when the required local proxy is unavailable.

The web-research extractor requires every factual claim to include source IDs and evidence. The Story Architect receives those source-backed facts and source URLs rather than being told to rely on its pretrained knowledge for external facts.

Privacy boundary: the search and source-fetch path can be kept local/Tor, but an LLM configured to a remote provider will still receive the user prompt and extracted research as part of generation. For end-to-end local privacy, use a local LLM endpoint such as Ollama together with the private gateway.

## LLM configuration

The engine uses an OpenAI-compatible `/chat/completions` endpoint so it can point at local or remote providers. Web research is a separate network path: the search query and fetched web content go through the local privacy gateway first. The default configuration is intended for a local Ollama-style endpoint:

- Base URL: `http://127.0.0.1:11434/v1`
- Model: `qwen3:8b`
- API key: optional

Every LLM generation requests streaming when supported by the provider. The Tauri backend emits token/delta events to the frontend, where the Live Generation Trace shows the exact system prompt, exact user prompt, model, live response, completion state and errors for every generation stage.

The Engine Settings panel exposes a separate editable system prompt for each stage:

- Story Architect
- Continuity Writer
- Scene Director
- LoRA Selector
- Image Prompt Generator
- Web Research Extractor

Web research settings include the local SearXNG URL, local Tor/SOCKS5 proxy, fail-closed proxy requirement, result/source limits, and research extractor prompt. These settings are persisted with the local application settings.


### Live service health

The top status strip and **Raphael Services** panel actively probe the configured Registry, SearXNG search API, and ComfyUI `/system_stats` endpoint. The UI reports CHECKING, ONLINE, OFFLINE, or DISABLED from those real API probes; these are not static labels.

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

Scene extraction and image prompt generation are implemented as independent stages so they can later connect to the Raphael Model Manager model/LoRA registry and ComfyUI workflow selector without changing story canon.


## ComfyUI workflow queue

The image builder queues a scene through ComfyUI's `/prompt` API, then monitors the job through ComfyUI WebSocket events with an HTTP history fallback. Live sampling/node progress is shown directly on the scene card. When the workflow completes, Raphael downloads the first generated image output into app data and displays it in the scene.

In Settings, paste a ComfyUI API-format workflow JSON template and use these placeholders inside string values:

- `{{POSITIVE_PROMPT}}`
- `{{NEGATIVE_PROMPT}}`
- `{{SEED}}`
- `{{STORY_ID}}`
- `{{SCENE_ID}}`
- `{{IMAGE_WIDTH}}`
- `{{IMAGE_HEIGHT}}`
- `{{IMAGE_SIZE}}`

The image-prompt LLM chooses one approved resolution for each scene (for example 1024×1024, 1216×832 or 832×1216). Raphael validates that choice and deterministically injects it into an Empty*Latent node or explicit size placeholders; a workflow without a valid size injection point is rejected instead of silently ignoring the requested size. The workflow itself still owns the checkpoint, LoRA, sampler and other generation nodes.

Use `"{{SEED}}"` as the complete value of a numeric seed field. Raphael converts that exact placeholder into a JSON number; embedded text such as `"seed={{SEED}}"` remains a string.

### Dynamic LoRA workflow builder

The workflow template must contain a `CheckpointLoaderSimple` or `CheckpointLoader` node. After LoRA selection, Raphael calls a deterministic workflow-builder tool with an ordered `lora_stack[]`.

The tool creates one ComfyUI `LoraLoader` node per selected LoRA and connects:

```
Checkpoint
   -> LoRA 1
      -> LoRA 2
         -> ...
            -> LoRA N
```

Both MODEL and CLIP are chained through every LoRA. The final MODEL/CLIP outputs replace the original checkpoint MODEL/CLIP references in the workflow, so there is no fixed number of LoRA slots.

The Story Generator supplies the stack in stable order: locked style LoRAs first, then selected character LoRAs, then the concept/pose LoRA. The selector chooses the LoRAs; the workflow builder only constructs the graph.

Automatic model/LoRA discovery from the Raphael Model Manager is handled through the shared Raphael Model Registry.


## Local settings and secrets

The optional LLM API key is stored in the application's local settings file and is not committed to the repository. Do not use a shared Windows account for credentials you need to keep private.


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


## Research visibility

When private research is enabled, the story view shows the actual search queries, fetched source titles/URLs/snippets, source-backed facts and the raw WEB RESEARCH LLM extractor response in the live trace. This makes it possible to inspect what search returned before the Story Architect uses the extracted facts.


SearXNG remains health-checked even when research is disabled, so the service panel reflects the API's actual availability rather than the feature toggle.


Feature-branch CI runs use the same concurrency group as pull-request runs, so the latest verification is the authoritative run instead of accumulating duplicate builds.


All actionable UI controls are rendered as semantic buttons, with runtime errors exposed through visible alert/error surfaces.
