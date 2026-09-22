# Raphael Story Generator

Raphael Story Generator is a local-first Tauri + React application for building persistent anime/manga stories and feeding them into a visual production pipeline.

## Current pipeline

```
User prompt
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

## LLM configuration

The engine uses an OpenAI-compatible `/chat/completions` endpoint so it can point at local or remote providers. The default configuration is intended for a local Ollama-style endpoint:

- Base URL: `http://127.0.0.1:11434/v1`
- Model: `qwen3:8b`
- API key: optional

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

The image builder can queue a scene through ComfyUI's `/prompt` API. In Settings, paste a ComfyUI API-format workflow JSON template and use these placeholders inside string values:

- `{{POSITIVE_PROMPT}}`
- `{{NEGATIVE_PROMPT}}`
- `{{SEED}}`
- `{{STORY_ID}}`
- `{{SCENE_ID}}`

The workflow itself owns the checkpoint, LoRA, sampler, resolution and other generation nodes. This keeps the story engine independent from a particular model library while still allowing a production workflow to be queued directly.

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
