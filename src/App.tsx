import { useEffect, useMemo, useState } from 'react';
import { api, subscribeToComfy, subscribeToLlm, subscribeToPipeline } from './tauri';
import type { AppSettings, AppState, Chapter, ComfyGenerationEvent, LlmGenerationEvent, PipelineEvent, RegistryCatalog, RegistryStatusDto, Scene, ServiceStatusBoard, Story, StoryVisualSetup } from './types';

function PulseMark() {
  return <div className="raphael-core" aria-label="Raphael"><span className="core-dot"/><i className="core-orbit orbit-a"/><i className="core-orbit orbit-b"/><i className="core-orbit orbit-c"/></div>;
}


function toErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object' && 'message' in error) return String((error as { message: unknown }).message);
  try { return JSON.stringify(error); } catch { return 'Unexpected error'; }
}

function BackgroundRaphael() {
  return <div className="background-raphael" aria-hidden="true"><PulseMark /></div>;
}

function splitTags(values: string[]) { return values.filter(Boolean).slice(0, 12); }

function formatDate(value: string) {
  const unix = value.match(/^unix:(\d+)$/);
  try {
    const date = unix ? new Date(Number(unix[1]) * 1000) : new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  } catch { return value; }
}

function StoryCard({ story, selected, onClick, disabled }: { story: AppState['stories'][number]; selected: boolean; onClick: () => void; disabled: boolean }) {
  return <button className={`story-card ${selected ? 'active' : ''}`} onClick={onClick} disabled={disabled}>
    <div className="story-card-top"><span className="story-index">STORY</span><span className="story-status">{story.chapter_count ? `${story.chapter_count} CH` : 'NEW'}</span></div>
    <strong>{story.title}</strong>
    <span className="story-card-meta">{story.scene_count} SCENES · {formatDate(story.updated_at)}</span>
  </button>;
}

function TagRow({ values, tone = '' }: { values: string[]; tone?: string }) {
  return <div className="tag-row">{splitTags(values).map(value => <span className={`tag ${tone}`} key={value}>{value}</span>)}</div>;
}

function NewStoryPanel({
  busy, prompt, setPrompt, onGenerate, registryStatus, catalog, checkpointId, setCheckpointId, styleLoraIds, setStyleLoraIds,
}: {
  busy: boolean; prompt: string; setPrompt: (v: string) => void; onGenerate: () => void;
  registryStatus: RegistryStatusDto; catalog: RegistryCatalog; checkpointId: string; setCheckpointId: (v: string) => void;
  styleLoraIds: string[]; setStyleLoraIds: (v: string[]) => void;
}) {
  const toggleStyle = (id: string) => {
    if (styleLoraIds.includes(id)) { setStyleLoraIds(styleLoraIds.filter(value => value !== id)); return; }
    if (styleLoraIds.length >= 2) return;
    setStyleLoraIds([...styleLoraIds, id]);
  };
  return <section className="hero-panel hud-panel">
    <div className="scan-corners" />
    <div className="eyebrow">RAPHAEL STORY ENGINE · VISUAL PROFILE FIRST</div>
    <h1>Turn a prompt into a living story.</h1>
    <p className="hero-copy">Describe the story, then choose the base checkpoint and optional style LoRAs for its entire visual identity. Character and concept/pose LoRAs are selected scene-by-scene while the chosen style remains locked.</p>
    <div className="visual-setup-grid">
      <label className="visual-select">BASE CHECKPOINT<select value={checkpointId} onChange={e => setCheckpointId(e.target.value)} disabled={busy || registryStatus.status !== 'on'}>
        <option value="">SELECT CHECKPOINT</option>{catalog.checkpoints.map(model => <option key={model.id} value={model.id}>{model.name}{model.base_model ? ' · ' + model.base_model : ''}</option>)}
      </select></label>
      <div className="style-picker"><div className="style-picker-head"><div><div className="section-head">LOCKED STYLE LORAS</div><span className="tiny">0–2 · FIXED FOR THIS STORY</span></div><span className="status-chip">{styleLoraIds.length}/2</span></div>
        {registryStatus.status !== 'on' ? <div className="style-picker-note">Waiting for Model Registry…</div> : catalog.loras.length === 0 ? <div className="style-picker-note">No LoRAs are registered.</div> : <div className="style-options">
          {catalog.loras.map(model => { const selected = styleLoraIds.includes(model.id); return <button type="button" className={'style-option ' + (selected ? 'selected' : '')} key={model.id} onClick={() => toggleStyle(model.id)} disabled={busy || (!selected && styleLoraIds.length >= 2)}><span className="style-option-check">{selected ? '✓' : '○'}</span><span><strong>{model.name}</strong><small>{model.base_model || 'BASE UNKNOWN'}</small></span></button>; })}
        </div>}
      </div>
    </div>
    <textarea className="story-prompt" value={prompt} onChange={event => setPrompt(event.target.value)} placeholder="Example: A dark fantasy anime about a quiet academy student who discovers that the rival she dislikes is protecting a secret tied to her family..." disabled={busy} rows={6}/>
    <div className="hero-actions"><button className="primary-btn" onClick={onGenerate} disabled={busy || !prompt.trim() || !checkpointId || registryStatus.status !== 'on'}>{busy ? 'BUILDING STORY BIBLE…' : 'GENERATE STORY'}</button><span className="tiny">STYLE LOCKED · SCENE LORAS AUTO-SELECTED · REGISTRY-VALIDATED</span></div>
  </section>;
}

function ServiceHealthPanel({ status, onRefresh, refreshing }: { status: ServiceStatusBoard; onRefresh: () => void; refreshing: boolean }) {
  const items = [status.registry, status.searxng, status.comfyui];
  const label = (value: ServiceStatusBoard['registry']['status']) => value.toUpperCase();
  return <section className="hud-panel inspector-panel service-health-panel">
    <div className="panel-title-row">
      <div><div className="section-head">RAPHAEL SERVICES</div><span className="tiny">LIVE API HEALTH · POLLED FROM THE ACTUAL ENDPOINTS</span></div>
      <button type="button" className="mini-btn" onClick={onRefresh} disabled={refreshing}>{refreshing ? 'CHECKING…' : 'RECHECK'}</button>
    </div>
    <div className="service-health-list">
      {items.map(item => (
        <div className="service-health-item" key={item.service} title={item.detail || item.url}>
          <span className={'service-health-dot ' + item.status}><i /></span>
          <div className="service-health-copy"><strong>{item.service.toUpperCase()}</strong><small>{label(item.status)} · {item.url}</small>{item.detail ? <em>{item.detail}</em> : null}</div>
        </div>
      ))}
    </div>
  </section>;
}

function RegistryPanel({ status, catalog, error }: { status: RegistryStatusDto; catalog: RegistryCatalog; error: string | null }) {
  const statusLabel = status.status === 'on' ? 'ON' : status.status === 'starting' ? 'STARTING' : 'OFF';
  const renderModelList = (models: RegistryCatalog['checkpoints']) => models.slice(0, 6).map(model => (
    <div className="registry-model" key={model.id} title={model.base_model ? `${model.name} · ${model.base_model}` : model.name}>
      <span>{model.name}</span>
      <small>{model.base_model || 'BASE UNKNOWN'}</small>
    </div>
  ));

  return <section className="hud-panel inspector-panel registry-panel">
    <div className="registry-panel-head">
      <div>
        <div className="section-head">MODEL REGISTRY</div>
        <span className="tiny">{status.url}</span>
      </div>
      <span className={`registry-state ${status.status}`}><i />{statusLabel}</span>
    </div>
    {status.status === 'on' ? <>
      <div className="registry-counts">
        <div><span>CHECKPOINTS</span><b>{catalog.checkpoint_total}</b></div>
        <div><span>LORAS</span><b>{catalog.lora_total}</b></div>
      </div>
      {error ? <div className="registry-error">{error}</div> : null}
      <div className="registry-list-group">
        <div className="registry-list-title">CHECKPOINTS</div>
        {catalog.checkpoints.length ? renderModelList(catalog.checkpoints) : <div className="registry-empty">No checkpoints registered.</div>}
      </div>
      <div className="registry-list-group">
        <div className="registry-list-title">LORAS</div>
        {catalog.loras.length ? renderModelList(catalog.loras) : <div className="registry-empty">No LoRAs registered.</div>}
      </div>
    </> : <div className="registry-offline">{status.detail || (status.status === 'starting' ? 'Starting Raphael Model Registry…' : 'Registry is not running.')}</div>}
  </section>;
}

function MetadataPanel({ story }: { story: Story }) {
  return <aside className="inspector">
    <section className="hud-panel inspector-panel">
      <div className="section-head">STORY PROFILE</div>
      <div className="profile-row"><span>DEMOGRAPHIC</span><b>{story.metadata.demographic || '—'}</b></div>
      <div className="profile-row"><span>CONTENT RATING</span><b>{story.metadata.content_rating || '—'}</b></div>
      <div className="profile-row"><span>SOURCE</span><b>{story.metadata.source_type || 'ORIGINAL'}</b></div>
      <div className="profile-row"><span>CHAPTERS</span><b>{story.chapters.length}</b></div>
      <div className="profile-row"><span>SCENES</span><b>{story.chapters.reduce((n, c) => n + c.scenes.length, 0)}</b></div>
    </section>
    <section className="hud-panel inspector-panel visual-profile-panel">
      <div className="section-head">VISUAL PROFILE</div>
      <div className="profile-row"><span>CHECKPOINT</span><b>{story.visual_config.checkpoint_name || '—'}</b></div>
      <div className="section-head muted-head">LOCKED STYLE</div>
      <div className="tag-row">{story.visual_config.style_loras.length ? story.visual_config.style_loras.map(lora => <span className="tag tone" key={lora.id}>{lora.name}</span>) : <span className="tiny">NONE SELECTED</span>}</div>
    </section>
    <section className="hud-panel inspector-panel">
      <div className="section-head">GENRE</div><TagRow values={story.metadata.genre}/>
      <div className="section-head muted-head">TAGS</div><TagRow values={story.metadata.tags} tone="dim"/>
    </section>
    <section className="hud-panel inspector-panel">
      <div className="section-head">CHARACTERS</div>
      <div className="character-list">{story.bible.characters.map(character => <div className="character-item" key={character.id}>
        <span className="avatar">{character.name.slice(0, 2).toUpperCase()}</span>
        <div><strong>{character.name}</strong><small>{character.role || 'character'}</small></div>
      </div>)}</div>
    </section>
    <section className="hud-panel inspector-panel">
      <div className="section-head">RELATIONSHIPS</div>
      <div className="relationship-list">
        {story.bible.relationships.length === 0 ? <div className="empty-small">No relationships recorded yet.</div> : story.bible.relationships.map((relationship, index) => {
          const from = story.bible.characters.find(c => c.id === relationship.source_character_id)?.name || relationship.source_character_id;
          const to = story.bible.characters.find(c => c.id === relationship.target_character_id)?.name || relationship.target_character_id;
          return <div className="relationship" key={`${relationship.source_character_id}-${relationship.target_character_id}-${index}`}><span>{from}</span><i>→</i><span>{to}</span><em>{relationship.relation_type}</em></div>;
        })}
      </div>
    </section>
  </aside>;
}

function IntroductionView({ story }: { story: Story }) {
  return <section className="hud-panel reading-panel introduction-panel">
    <div className="section-head">INTRODUCTION</div>
    <div className="chapter-text">{story.introduction.split(/\n\s*\n/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}</div>
  </section>;
}

function ResearchPanel({ story, researchTrace }: { story: Story; researchTrace: GenerationTrace | null }) {
  const research = story.research;
  return <section className="hud-panel inspector-panel research-panel">
    <div className="panel-title-row">
      <div>
        <div className="section-head">PRIVATE WEB RESEARCH</div>
        <span className="tiny">LIVE SEARXNG RESULTS · TOR FETCH · LLM EXTRACTION</span>
      </div>
      <span className="status-chip">{research.sources.length} SOURCES · {research.facts.length} FACTS</span>
    </div>

    {research.queries.length ? <div className="research-query-block">
      <div className="section-head muted-head">SEARCH QUERIES</div>
      <div className="tag-row">{research.queries.map(query => <span className="tag dim" key={query}>{query}</span>)}</div>
    </div> : null}

    {research.sources.length ? (
      <div className="research-sources">
        <div className="section-head muted-head">SEARCH RESULTS / FETCHED SOURCES</div>
        {research.sources.slice(0, 10).map(source => (
          <article className="research-source research-result" key={source.id}>
            <div className="research-source-head"><span className="research-source-id">{source.id}</span><strong>{source.title || source.url}</strong></div>
            <small>{source.url}</small>
            {source.snippet ? <p className="research-snippet">{source.snippet}</p> : null}
          </article>
        ))}
      </div>
    ) : <div className="registry-empty">No external research sources were returned.</div>}

    <div className="research-facts">
      <div className="section-head muted-head">SOURCE-BACKED FACTS</div>
      {research.facts.length ? research.facts.slice(0, 10).map((fact, index) => (
        <article className="research-fact" key={index}>
          <strong>{fact.claim}</strong>
          <small>{fact.source_ids.join(' · ')} · {fact.confidence.toUpperCase()}</small>
          <p>{fact.evidence}</p>
        </article>
      )) : <div className="registry-empty">No supported facts were extracted.</div>}
    </div>

    {researchTrace ? <details className="research-llm-trace">
      <summary><span className="section-head">RAW RESEARCH EXTRACTOR OUTPUT</span><span className="tiny">{researchTrace.status.toUpperCase()}</span></summary>
      <pre className="trace-box response">{researchTrace.response || '(no response captured)'}</pre>
      {researchTrace.error ? <div className="error-box">{researchTrace.error}</div> : null}
    </details> : null}
  </section>;
}


function ChapterView({ story, chapter, onExtractScenes, extracting, onBuildPrompt, onViewPrompt, onQueueImage, buildingPrompt, queueing, progress, images }: { story: Story; chapter: Chapter; onExtractScenes: () => void; extracting: boolean; onBuildPrompt: (scene: Scene) => void; onViewPrompt: (scene: Scene) => void; onQueueImage: (scene: Scene) => void; buildingPrompt: string | null; queueing: string | null; progress: Record<string, ComfyGenerationEvent>; images: Record<string, string>; }) {
  return <div className="chapter-view">
    <div className="chapter-header hud-panel">
      <div><div className="eyebrow">CHAPTER {String(chapter.number).padStart(2, '0')}</div><h2>{chapter.title}</h2></div>
      <div className="chapter-header-actions"><span className="status-chip">CANONICAL</span><button className="secondary-btn" onClick={onExtractScenes} disabled={extracting}>{extracting ? 'EXTRACTING…' : chapter.scenes.length ? 'REBUILD SCENES' : 'EXTRACT SCENES'}</button></div>
    </div>
    <section className="hud-panel reading-panel">
      <div className="section-head">SUMMARY</div><p className="chapter-summary">{chapter.summary}</p>
      <div className="section-head">STORY</div>
      <div className="chapter-text">{chapter.text.split(/\n\s*\n/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}</div>
    </section>
    <section className="hud-panel continuity-panel">
      <div className="section-head">CONTINUITY SIGNALS</div>
      <div className="signal-grid">{(chapter.events.length ? chapter.events : ['No structured events recorded.']).slice(0, 8).map((event, index) => <div className="signal" key={index}><span>{String(index + 1).padStart(2, '0')}</span>{event}</div>)}</div>
      {chapter.user_directive ? <div className="directive"><span>USER DIRECTIVE</span><p>{chapter.user_directive}</p></div> : null}
    </section>
    <section className="hud-panel scenes-panel">
      <div className="panel-title-row"><div><div className="section-head">VISUAL SCENES</div><span className="tiny">ONE IMAGEABLE STORY BEAT PER SCENE</span></div><span className="status-chip">{chapter.scenes.length} SCENES</span></div>
      {chapter.scenes.length === 0 ? <div className="scene-empty">Extract this chapter into visual beats for the manga/image pipeline.</div> : <div className="scene-grid">{chapter.scenes.map(scene => <article className="scene-card" key={scene.id}>
        <div className="scene-index">SCENE {String(scene.order).padStart(2, '0')}</div><h3>{scene.description}</h3>
        <div className="scene-meta"><span>{scene.location || 'UNKNOWN LOCATION'}</span><span>{scene.time || 'TIME UNSPECIFIED'}</span></div>
        <p className="scene-action">{scene.action}</p>
        <div className="scene-image-size">OUTPUT {scene.image_width}×{scene.image_height}</div>
        {scene.selected_loras.length ? <div className="scene-lora-strip">{scene.selected_loras.map(lora => <span className="scene-lora-chip" key={lora.id} title={lora.reason}>{lora.role === 'character' ? 'CHAR' : 'POSE'} · {lora.name}</span>)}</div> : null}
        {images[scene.id] ? <img className="scene-image" src={images[scene.id]} alt={scene.description} /> : null}
        {(() => {
          const currentProgress = progress[scene.id];
          if (!currentProgress || (currentProgress.status !== 'queued' && currentProgress.status !== 'running')) return null;
          const percent = currentProgress.progress;
          return <div className="scene-progress">
            <div className="scene-progress-head"><span>{currentProgress.message}</span><b>{percent == null ? '—' : Math.round(percent) + '%'}</b></div>
            <div className="scene-progress-track"><div className={'scene-progress-fill ' + (percent == null ? 'indeterminate' : '')} style={percent == null ? undefined : { width: Math.max(0, Math.min(100, percent)) + '%' }} /></div>
            <small>{currentProgress.current_node ? 'NODE ' + currentProgress.current_node : currentProgress.queue_remaining != null ? 'QUEUE REMAINING ' + currentProgress.queue_remaining : 'COMFYUI EXECUTION'}</small>
          </div>;
        })()}
        {scene.image_error ? <div className="scene-error">{scene.image_error}</div> : null}
        <div className="scene-footer"><span className={'scene-status ' + scene.image_status}>{scene.image_status.replace('_', ' ').toUpperCase()}</span><div className="scene-actions"><button className="mini-btn" onClick={() => scene.positive_prompt ? onViewPrompt(scene) : onBuildPrompt(scene)} disabled={buildingPrompt === scene.id || queueing === scene.id}>{buildingPrompt === scene.id ? 'BUILDING…' : scene.positive_prompt ? 'VIEW PROMPT' : 'BUILD IMAGE PROMPT'}</button>{scene.positive_prompt ? <button className="mini-btn queue-btn" onClick={() => onQueueImage(scene)} disabled={queueing === scene.id}>{queueing === scene.id ? 'QUEUING…' : scene.image_status === 'queued' || scene.image_status === 'running' ? 'REQUEUE IMAGE' : 'QUEUE IMAGE'}</button> : null}</div></div>
      </article>)}</div>}
    </section>
  </div>;
}

type GenerationTrace = {
  generation_id: string;
  stage: string;
  status: LlmGenerationEvent['status'];
  model: string;
  system_prompt: string;
  user_prompt: string;
  response: string;
  error: string | null;
};

function GenerationMonitor({
  generations,
  pipeline,
  onClear,
}: {
  generations: GenerationTrace[];
  pipeline: PipelineEvent[];
  onClear: () => void;
}) {
  const active = generations.find(g => g.status === 'started' || g.status === 'token') || null;
  const stageLabel = (stage: string) => stage.replaceAll('_', ' ').toUpperCase();

  return <section className="hud-panel generation-monitor">
    <div className="panel-title-row">
      <div><div className="section-head">LIVE GENERATION TRACE</div><span className="tiny">{active ? ('STREAMING · ' + stageLabel(active.stage)) : 'EVERY LLM CALL · TOKEN STREAM · PIPELINE EVENTS'}</span></div>
      <div className="trace-actions"><span className={'trace-live ' + (active ? 'active' : '')}><i />{active ? 'LIVE' : 'IDLE'}</span><button className="mini-btn" onClick={onClear} disabled={!generations.length && !pipeline.length}>CLEAR</button></div>
    </div>

    {pipeline.length ? <div className="pipeline-log">
      {pipeline.slice(-8).map(event => <div className={'pipeline-log-item ' + event.status} key={event.event_id}>
        <span>{stageLabel(event.stage)}</span><strong>{event.message}</strong>
      </div>)}
    </div> : null}

    <div className="generation-list">
      {generations.length === 0 ? <div className="scene-empty">No generations yet. Start a story, chapter, scene extraction, LoRA selection, or image prompt to watch the live trace.</div> : generations.slice().reverse().map(generation => (
        <details className={'generation-card ' + ((generation.status === 'started' || generation.status === 'token') ? 'streaming' : '')} key={generation.generation_id} open={generation.generation_id === active?.generation_id}>
          <summary>
            <span className={'generation-dot ' + generation.status} />
            <span className="generation-stage">{stageLabel(generation.stage)}</span>
            <span className="generation-model">{generation.model}</span>
            <span className="generation-status">{generation.status.toUpperCase()}</span>
          </summary>
          <div className="generation-body">
            <div className="trace-block">
              <div className="section-head">SYSTEM PROMPT</div>
              <pre className="trace-box">{generation.system_prompt}</pre>
            </div>
            <div className="trace-block">
              <div className="section-head">USER PROMPT</div>
              <pre className="trace-box">{generation.user_prompt}</pre>
            </div>
            <div className="trace-block">
              <div className="section-head">LLM RESPONSE {(generation.status === 'started' || generation.status === 'token') ? '· STREAMING' : ''}</div>
              <pre className="trace-box response">{generation.response}{(generation.status === 'started' || generation.status === 'token') ? '▌' : ''}</pre>
            </div>
            {generation.error ? <div className="error-box">{generation.error}</div> : null}
          </div>
        </details>
      ))}
    </div>
  </section>;
}

function SettingsOverlay({ settings, llmApiKeyConfigured, onSave, onClearApiKey, onClose, onError }: { settings: AppSettings; llmApiKeyConfigured: boolean; onSave: (settings: AppSettings) => Promise<void>; onClearApiKey: () => Promise<void>; onClose: () => void; onError: (message: string) => void }) {
  const [draft, setDraft] = useState(settings);
  const [busy, setBusy] = useState(false);
  const [researchTesting, setResearchTesting] = useState(false);
  const [researchTestResult, setResearchTestResult] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    setSaveError(null);
    try {
      await onSave(draft);
      onClose();
    } catch (error) {
      const message = toErrorMessage(error);
      setSaveError(message);
      onError(message);
    } finally {
      setBusy(false);
    }
  };

  const testPrivateResearch = async () => {
    setResearchTesting(true);
    setResearchTestResult(null);
    try {
      setResearchTestResult(await api.testPrivateWebResearch());
    } catch (error) {
      setResearchTestResult(String(error));
    } finally {
      setResearchTesting(false);
    }
  };
  return <div className="overlay" onMouseDown={onClose}><section className="settings-panel hud-panel" onMouseDown={e => e.stopPropagation()}>
    <header className="settings-header"><div><div className="eyebrow">RAPHAEL CORE</div><h2>ENGINE SETTINGS</h2></div><button className="settings-close" onClick={onClose}>×</button></header>
    <div className="settings-scroll">
      <div className="section-head">OPENAI-COMPATIBLE LLM</div>
      <label>Base URL<input value={draft.llm_base_url} onChange={e => setDraft({ ...draft, llm_base_url: e.target.value })} placeholder="http://127.0.0.1:11434/v1"/></label>
      <label>Model<input value={draft.llm_model} onChange={e => setDraft({ ...draft, llm_model: e.target.value })} placeholder="qwen3:8b"/></label>
      <label>API key<input type="password" value={draft.llm_api_key} onChange={e => setDraft({ ...draft, llm_api_key: e.target.value })} placeholder={llmApiKeyConfigured ? 'Stored securely · leave blank to keep current key' : 'Optional for local endpoints'}/>{llmApiKeyConfigured ? <button type="button" className="mini-btn settings-clear-key" onClick={() => void onClearApiKey()}>CLEAR STORED KEY</button> : null}</label>
      <label>Temperature<input type="number" min="0" max="2" step="0.1" value={draft.temperature} onChange={e => setDraft({ ...draft, temperature: Number(e.target.value) || 0 })}/></label>
      <div className="section-head setting-gap">SYSTEM PROMPTS · FULLY CUSTOMIZABLE</div>
      <small className="settings-hint">These prompts are sent as the system message for each LLM generation stage. They are persisted with the application settings and shown live in the Generation Trace.</small>
      <label>Story Architect<small className="settings-hint">Creates the story bible, opening chapter, characters, relationships, and visual canon.</small><textarea className="system-prompt-input" value={draft.story_architect_system_prompt} onChange={e => setDraft({ ...draft, story_architect_system_prompt: e.target.value })}/></label>
      <label>Continuity Writer<small className="settings-hint">Generates subsequent chapters while preserving established canon.</small><textarea className="system-prompt-input" value={draft.continuity_writer_system_prompt} onChange={e => setDraft({ ...draft, continuity_writer_system_prompt: e.target.value })}/></label>
      <label>Scene Director<small className="settings-hint">Splits a chapter into imageable visual beats.</small><textarea className="system-prompt-input" value={draft.scene_director_system_prompt} onChange={e => setDraft({ ...draft, scene_director_system_prompt: e.target.value })}/></label>
      <label>LoRA Selector<small className="settings-hint">Chooses character and concept/pose LoRAs from Registry metadata.</small><textarea className="system-prompt-input" value={draft.lora_selector_system_prompt} onChange={e => setDraft({ ...draft, lora_selector_system_prompt: e.target.value })}/></label>
      <label>Image Prompt Generator<small className="settings-hint">Builds the final positive and negative diffusion prompts from scene facts and LoRA activation prompts.</small><textarea className="system-prompt-input" value={draft.image_prompt_generator_system_prompt} onChange={e => setDraft({ ...draft, image_prompt_generator_system_prompt: e.target.value })}/></label>
      <div className="section-head setting-gap">PRIVATE WEB RESEARCH</div>
      <small className="settings-hint">Research is fail-closed for privacy: Raphael accepts only a local SearXNG gateway and, by default, requires a local SOCKS5/Tor proxy. Search providers and fetched source sites are contacted through the Tor path; no public SearXNG service is used.</small>
      <label className="toggle-setting"><input type="checkbox" checked={draft.web_research_enabled} onChange={e => setDraft({ ...draft, web_research_enabled: e.target.checked })}/> Enable web research before story generation</label>
      <label>Local SearXNG URL<input value={draft.web_search_url} onChange={e => setDraft({ ...draft, web_search_url: e.target.value })} placeholder="http://127.0.0.1:8080"/></label>
      <label>Local SOCKS/Tor proxy<input value={draft.web_proxy_url} onChange={e => setDraft({ ...draft, web_proxy_url: e.target.value })} placeholder="socks5h://127.0.0.1:9050"/></label>
      <div className="toggle-setting">TOR PROXY REQUIREMENT · ENFORCED</div>
      <div className="settings-grid-two">
        <label>Max search results<input type="number" min="1" max="12" value={draft.web_search_max_results} onChange={e => setDraft({ ...draft, web_search_max_results: Number(e.target.value) || 1 })}/></label>
        <label>Max page text<input type="number" min="2000" max="12000" step="500" value={draft.web_fetch_max_chars} onChange={e => setDraft({ ...draft, web_fetch_max_chars: Number(e.target.value) || 2000 })}/></label>
      </div>
      <label>Research context limit<input type="number" min="8000" max="48000" step="1000" value={draft.web_context_max_chars} onChange={e => setDraft({ ...draft, web_context_max_chars: Number(e.target.value) || 8000 })}/></label>
      <label>Research Extractor<small className="settings-hint">The extractor receives fetched page text and must cite every extracted fact by source ID.</small><textarea className="system-prompt-input" value={draft.web_research_system_prompt} onChange={e => setDraft({ ...draft, web_research_system_prompt: e.target.value })}/></label>
      <div className="research-test-row"><button type="button" className="secondary-btn" onClick={() => void testPrivateResearch()} disabled={researchTesting}>{researchTesting ? 'CHECKING…' : 'TEST PRIVATE GATEWAY'}</button>{researchTestResult ? <span className="tiny">{researchTestResult}</span> : null}</div>

      <div className="section-head setting-gap">COMFYUI</div>
      <label>API URL<input value={draft.comfyui_url} onChange={e => setDraft({ ...draft, comfyui_url: e.target.value })} placeholder="http://127.0.0.1:8188"/></label>
      <label>API workflow template<small className="settings-hint">Use POSITIVE_PROMPT, NEGATIVE_PROMPT, SEED, STORY_ID, SCENE_ID and CHECKPOINT placeholders, plus IMAGE_WIDTH and IMAGE_HEIGHT (or IMAGE_SIZE). Raphael requires the workflow to expose an image-size injection point: an Empty*Latent node with width/height inputs or the size placeholders. The workflow must contain a CheckpointLoaderSimple or CheckpointLoader node. Raphael's workflow-builder inserts one LoraLoader node per selected LoRA and chains MODEL + CLIP through the full stack.</small><textarea className="workflow-input" value={draft.comfyui_workflow_json} onChange={e => setDraft({ ...draft, comfyui_workflow_json: e.target.value })} placeholder='Paste a ComfyUI API workflow JSON template here...'/></label>
    </div>
    {saveError ? <div className="settings-inline-error error-box">{saveError}</div> : null}
    <footer className="settings-footer"><button className="secondary-btn" onClick={onClose}>CANCEL</button><button className="primary-btn" onClick={() => void save()} disabled={busy}>{busy ? 'SAVING…' : 'SAVE SETTINGS'}</button></footer>
  </section></div>;
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null); const [story, setStory] = useState<Story | null>(null);
  const [prompt, setPrompt] = useState(''); const [directive, setDirective] = useState('');
  const [busy, setBusy] = useState(false); const [extracting, setExtracting] = useState(false); const [buildingPrompt, setBuildingPrompt] = useState<string | null>(null); const [queueing, setQueueing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null); const [settingsOpen, setSettingsOpen] = useState(false); const [promptPreview, setPromptPreview] = useState<Scene | null>(null);
  const [registryStatus, setRegistryStatus] = useState<RegistryStatusDto>({ status: 'starting', url: 'http://127.0.0.1:43217', detail: 'Starting Raphael Model Registry…' });
  const [registryCatalog, setRegistryCatalog] = useState<RegistryCatalog>({ checkpoints: [], checkpoint_total: 0, loras: [], lora_total: 0 });
  const [registryCatalogError, setRegistryCatalogError] = useState<string | null>(null);
  const [checkpointId, setCheckpointId] = useState('');
  const [styleLoraIds, setStyleLoraIds] = useState<string[]>([]);
  const [generationTraces, setGenerationTraces] = useState<GenerationTrace[]>([]);
  const [pipelineTrace, setPipelineTrace] = useState<PipelineEvent[]>([]);
  const [comfyProgress, setComfyProgress] = useState<Record<string, ComfyGenerationEvent>>({});
  const [sceneImages, setSceneImages] = useState<Record<string, string>>({});
  const [selectedChapterNumber, setSelectedChapterNumber] = useState<number | null>(null);
  const [serviceStatus, setServiceStatus] = useState<ServiceStatusBoard>({
    registry: { service: 'registry', status: 'checking', url: 'http://127.0.0.1:43217', detail: 'Checking Registry API…' },
    searxng: { service: 'searxng', status: 'checking', url: 'http://127.0.0.1:8080', detail: 'Checking SearXNG API…' },
    comfyui: { service: 'comfyui', status: 'checking', url: 'http://127.0.0.1:8188', detail: 'Checking ComfyUI API…' },
  });
  const [serviceRefreshing, setServiceRefreshing] = useState(false);


  const loadState = async () => {
    const next = await api.getState();
    setState(next);
    return next;
  };
  const selectStory = async (id: string) => {
    setError(null);
    try {
      setDirective('');
      setPromptPreview(null);
      setSceneImages({});
      setComfyProgress({});
      const next = await api.getStory(id);
      setStory(next);
      setSelectedChapterNumber(next.chapters[next.chapters.length - 1]?.number ?? null);
    } catch (e) {
      setError(toErrorMessage(e));
    }
  };
  useEffect(() => {
    let disposed = false;
    let unlistenLlm: (() => void) | null = null;
    let unlistenPipeline: (() => void) | null = null;
    let unlistenComfy: (() => void) | null = null;

    void subscribeToLlm(event => {
      if (disposed) return;
      setGenerationTraces(current => {
        const existing = current.find(item => item.generation_id === event.generation_id);
        if (event.status === 'started') {
          if (existing) return current;
          const next: GenerationTrace = {
            generation_id: event.generation_id,
            stage: event.stage,
            status: event.status,
            model: event.model,
            system_prompt: event.system_prompt || '',
            user_prompt: event.user_prompt || '',
            response: '',
            error: null,
          };
          return [...current, next];
        }
        if (!existing) return current;

        return current.map(item => {
          if (item.generation_id !== event.generation_id) return item;
          return {
            ...item,
            status: event.status,
            response: event.response ?? (event.delta ? item.response + event.delta : item.response),
            error: event.error || item.error,
          };
        });
      });
    }).then(unlisten => {
      if (disposed) unlisten();
      else unlistenLlm = unlisten;
    });

    void subscribeToPipeline(event => {
      if (disposed) return;
      setPipelineTrace(current => [...current, event]);
    }).then(unlisten => {
      if (disposed) unlisten();
      else unlistenPipeline = unlisten;
    });

    void subscribeToComfy(event => {
      if (disposed) return;
      setComfyProgress(current => ({ ...current, [event.scene_id]: event }));
      if (event.status === 'completed' || event.status === 'failed') {
        void api.getStory(event.story_id).then(next => {
          if (disposed) return;
          setStory(current => current?.id === next.id ? next : current);
        }).catch(error => setError(`ComfyUI scene refresh failed: ${toErrorMessage(error)}`));
      }
      if (event.status === 'completed') {
        void api.getSceneImage(event.story_id, event.chapter_number, event.scene_id)
          .then(image => {
            if (disposed || !image) return;
            setSceneImages(current => ({ ...current, [event.scene_id]: image }));
          })
          .catch(() => {});
      }
    }).then(unlisten => {
      if (disposed) unlisten();
      else unlistenComfy = unlisten;
    });

    return () => {
      disposed = true;
      unlistenLlm?.();
      unlistenPipeline?.();
      unlistenComfy?.();
    };
  }, []);

  useEffect(() => { void loadState().catch(e => setError(toErrorMessage(e))); }, []);
  useEffect(() => {
    let disposed = false;
    const apply = (next: RegistryStatusDto) => {
      if (!disposed) setRegistryStatus(next);
    };

    setRegistryStatus(current => ({ ...current, status: 'starting', detail: 'Starting Raphael Model Registry…' }));
    void api.ensureRegistry()
      .then(apply)
      .then(() => refreshServiceStatus())
      .catch(error => {
        if (!disposed) {
          setRegistryStatus({
            status: 'off',
            url: 'http://127.0.0.1:43217',
            detail: String(error),
          });
        }
      });

    const timer = window.setInterval(() => {
      void api.getRegistryStatus().then(apply).catch(error => {
        if (!disposed) {
          setRegistryStatus(current => ({
            ...current,
            status: 'off',
            detail: String(error),
          }));
        }
      });
    }, 1500);

    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);


  useEffect(() => {
    if (!story || selectedChapterNumber == null) return;
    const chapter = story.chapters.find(item => item.number === selectedChapterNumber);
    if (!chapter) return;
    let disposed = false;
    void Promise.all(chapter.scenes.filter(scene => scene.image_status === 'generated').map(async scene => {
      try {
        const image = await api.getSceneImage(story.id, chapter.number, scene.id);
        if (!disposed && image) setSceneImages(current => ({ ...current, [scene.id]: image }));
      } catch {
        // A deleted image should not prevent the chapter from loading.
      }
    }));
    return () => { disposed = true; };
  }, [story?.id, selectedChapterNumber]);

  const refreshServiceStatus = async () => {
    setServiceRefreshing(true);
    try {
      setServiceStatus(await api.getServiceStatus());
    } catch (e) {
      setServiceStatus(current => ({
        registry: { ...current.registry, status: 'offline', detail: toErrorMessage(e) },
        searxng: { ...current.searxng, status: 'offline', detail: toErrorMessage(e) },
        comfyui: { ...current.comfyui, status: 'offline', detail: toErrorMessage(e) },
      }));
    } finally {
      setServiceRefreshing(false);
    }
  };

  useEffect(() => {
    void refreshServiceStatus();
    const timer = window.setInterval(() => { void refreshServiceStatus(); }, 3000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (registryStatus.status !== 'on') {
      setRegistryCatalog({ checkpoints: [], checkpoint_total: 0, loras: [], lora_total: 0 });
      setRegistryCatalogError(null);
      return;
    }

    let disposed = false;
    const loadModels = async () => {
      try {
        const catalog = await api.getRegistryModels();
        if (disposed) return;
        setRegistryCatalog(catalog);
        setRegistryCatalogError(null);
      } catch (error) {
        if (disposed) return;
        setRegistryCatalog({ checkpoints: [], checkpoint_total: 0, loras: [], lora_total: 0 });
        setRegistryCatalogError(String(error));
      }
    };

    void loadModels();
    const timer = window.setInterval(() => void loadModels(), 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [registryStatus.status]);

  useEffect(() => {
    if (registryStatus.status === 'on' && !checkpointId && registryCatalog.checkpoints.length > 0) {
      setCheckpointId(registryCatalog.checkpoints[0].id);
    }
  }, [registryStatus.status, registryCatalog.checkpoints, checkpointId]);

  useEffect(() => { if (!story && state?.stories[0]) void selectStory(state.stories[0].id); }, [state?.stories]);

  const generateStory = async () => {
    if (!prompt.trim() || busy || !checkpointId || registryStatus.status !== 'on') return;
    setBusy(true); setError(null);
    try {
      const visualSetup: StoryVisualSetup = { checkpoint_id: checkpointId, style_lora_ids: styleLoraIds.slice(0, 2) };
      const next = await api.createStory(prompt.trim(), visualSetup);
      setStory(next);
      setSelectedChapterNumber(next.chapters[next.chapters.length - 1]?.number ?? null);
      setPrompt('');
      await loadState();
    } catch (e) { setError(toErrorMessage(e)); } finally { setBusy(false); }
  };
  const generateNext = async () => { if (!story || busy) return; setBusy(true); setError(null); try { const next = await api.generateNextChapter(story.id, directive.trim()); setStory(next); setSelectedChapterNumber(next.chapters[next.chapters.length - 1]?.number ?? null); setDirective(''); await loadState(); } catch (e) { setError(toErrorMessage(e)); } finally { setBusy(false); } };
  const extractScenes = async (chapter: Chapter) => {
    if (!story || extracting) return;
    if (chapter.scenes.length > 0 && !window.confirm('Rebuild scenes? Existing scene prompts, queue IDs, and image state for this chapter will be replaced.')) {
      return;
    }
    setExtracting(true);
    setError(null);
    try {
      await api.extractScenes(story.id, chapter.number);
      setStory(await api.getStory(story.id));
      await loadState();
    } catch (e) {
      setError(toErrorMessage(e));
    } finally {
      setExtracting(false);
    }
  };
  const buildPrompt = async (scene: Scene) => { if (!story || buildingPrompt) return; const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id)); if (!chapter) return; setBuildingPrompt(scene.id); setError(null); try { setStory(await api.buildScenePrompt(story.id, chapter.number, scene.id)); await loadState(); } catch (e) { setError(toErrorMessage(e)); } finally { setBuildingPrompt(null); } };
  const viewPrompt = (scene: Scene) => setPromptPreview(scene);
  const queueImage = async (scene: Scene) => { if (!story || queueing) return; const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id)); if (!chapter) return; setQueueing(scene.id); setError(null); try { setStory(await api.queueSceneImage(story.id, chapter.number, scene.id)); await loadState(); } catch (e) { setError(toErrorMessage(e)); } finally { setQueueing(null); } };

  const currentChapter = story
    ? story.chapters.find(chapter => chapter.number === selectedChapterNumber) || story.chapters[story.chapters.length - 1] || null
    : null;
  const totalScenes = useMemo(() => story?.chapters.reduce((n, c) => n + c.scenes.length, 0) || 0, [story]);
  const researchTrace = generationTraces.slice().reverse().find(trace => trace.stage.includes('research')) || null;

  if (!state) return (
    <div className="setup-shell">
      <PulseMark/>
      <div className="loading-card hud-panel">
        <div className="eyebrow">RAPHAEL STORY GENERATOR</div>
        {error ? (
          <>
            <h1>STORY ENGINE FAILED TO INITIALIZE</h1>
            <div className="error-box">{error}</div>
            <button className="primary-btn" onClick={() => {
              setError(null);
              void loadState().catch(e => setError(toErrorMessage(e)));
            }}>RETRY</button>
          </>
        ) : (
          <>
            <h1>INITIALIZING STORY ENGINE…</h1>
            <span className="pulse-line"/>
          </>
        )}
      </div>
    </div>
  );

  return <div className="app-shell">
    <BackgroundRaphael />
    <header className="topbar">
      <div className="brand"><PulseMark/><div><div className="brand-title">RAPHAEL</div><div className="brand-subtitle">STORY GENERATOR</div></div></div>
      <div className="top-status"><span className={state.llm_configured ? 'status-ok' : 'status-warn'}>LLM {state.llm_configured ? 'READY' : 'NOT CONFIGURED'}</span><span className={`registry-health ${serviceStatus.registry.status}`} title={serviceStatus.registry.detail || serviceStatus.registry.url}><i />REGISTRY {serviceStatus.registry.status.toUpperCase()}</span><span className={'service-inline ' + serviceStatus.searxng.status}>SEARXNG {serviceStatus.searxng.status.toUpperCase()}</span><span className={'service-inline ' + serviceStatus.comfyui.status}>COMFYUI {serviceStatus.comfyui.status.toUpperCase()}</span><span>SCENES {totalScenes}</span><span className={generationTraces.some(g => g.status === 'started' || g.status === 'token') ? 'status-ok' : ''}>TRACE {generationTraces.length}</span><button className="icon-btn" onClick={() => setSettingsOpen(true)} title="Engine settings">⚙</button></div>
    </header>

    <div className="workspace">
      <aside className="sidebar">
        <div className="sidebar-head"><div><div className="eyebrow">LIBRARY</div><h2>STORIES</h2></div><button className="square-btn" disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing)} onClick={() => { setStory(null); setPrompt(''); setDirective(''); setPromptPreview(null); }}>+</button></div>
        <div className="story-list">{state.stories.length === 0 ? <div className="empty-sidebar">No stories yet.<br/>Create the first one from the story prompt.</div> : state.stories.map(item => <StoryCard key={item.id} story={item} selected={story?.id === item.id} disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing)} onClick={() => void selectStory(item.id)}/>)}</div>
        <div className="sidebar-foot">LOCAL-FIRST · CANON SAVED TO APP DATA</div>
      </aside>

      <main className="main-panel">
        {error ? <div className="error-banner error-box" role="alert"><strong>PIPELINE ERROR</strong><span>{error}</span><button type="button" className="mini-btn" onClick={() => setError(null)}>DISMISS</button></div> : null}
        {!story ? <NewStoryPanel busy={busy} prompt={prompt} setPrompt={setPrompt} onGenerate={() => void generateStory()} registryStatus={registryStatus} catalog={registryCatalog} checkpointId={checkpointId} setCheckpointId={setCheckpointId} styleLoraIds={styleLoraIds} setStyleLoraIds={setStyleLoraIds}/> : <>
          <section className="story-header hud-panel">
            <div><div className="eyebrow">STORY BIBLE · {story.id.slice(0, 8).toUpperCase()}</div><h1>{story.title}</h1><p>{story.bible.premise}</p><label className="chapter-selector">VIEW CHAPTER<select value={selectedChapterNumber ?? story.chapters[story.chapters.length - 1]?.number ?? 1} onChange={e => setSelectedChapterNumber(Number(e.target.value))}>{story.chapters.map(chapter => <option key={chapter.number} value={chapter.number}>CHAPTER {chapter.number} · {chapter.title}</option>)}</select></label></div>
            <div className="story-header-tags"><TagRow values={story.metadata.tone} tone="tone"/></div>
          </section>
          <IntroductionView story={story}/>
          {currentChapter ? <ChapterView story={story} chapter={currentChapter} onExtractScenes={() => void extractScenes(currentChapter)} extracting={extracting} onBuildPrompt={scene => void buildPrompt(scene)} onViewPrompt={viewPrompt} onQueueImage={queueImage} buildingPrompt={buildingPrompt} queueing={queueing} progress={comfyProgress} images={sceneImages}/> : null}
          <section className="hud-panel continuation-panel">
            <div className="section-head">CONTINUE THE STORY</div>
            <h2>Chapter {story.chapters.length + 1}</h2>
            <p>Optional: tell Raphael what you want changed, introduced, emphasized or avoided. Leave it empty for a natural continuation.</p>
            <textarea value={directive} onChange={e => setDirective(e.target.value)} placeholder="Example: Introduce a new female rival, keep the mood tense, and don't reveal the villain yet." rows={4} disabled={busy}/>
            <div className="continue-actions"><button className="primary-btn" onClick={() => void generateNext()} disabled={busy}>{busy ? 'WRITING NEXT CHAPTER…' : `GENERATE CHAPTER ${story.chapters.length + 1}`}</button><span className="tiny">DIRECTIVE IS A REQUEST · ESTABLISHED CANON REMAINS AUTHORITATIVE</span></div>
          </section>
        </>}
      </main>

      <aside className="inspector">
        <ServiceHealthPanel status={serviceStatus} onRefresh={() => void refreshServiceStatus()} refreshing={serviceRefreshing}/>
        <RegistryPanel status={registryStatus} catalog={registryCatalog} error={registryCatalogError}/>
        <GenerationMonitor
          generations={generationTraces}
          pipeline={pipelineTrace}
          onClear={() => { setGenerationTraces([]); setPipelineTrace([]); }}
        />
        {story ? <ResearchPanel story={story} researchTrace={researchTrace}/> : null}
        {story ? <MetadataPanel story={story}/> : <section className="hud-panel inspector-panel placeholder-inspector"><div className="section-head">PIPELINE</div><div className="pipeline-step active"><b>01</b><span>STORY BIBLE</span></div><div className="pipeline-step"><b>02</b><span>CHAPTERS</span></div><div className="pipeline-step"><b>03</b><span>SCENE EXTRACTION</span></div><div className="pipeline-step"><b>04</b><span>IMAGE BUILDER</span></div><div className="pipeline-note">The story engine is designed so scene extraction and ComfyUI image generation can run without changing the story canon.</div></section>}
      </aside>
    </div>

    {promptPreview ? <div className="overlay" onMouseDown={() => setPromptPreview(null)}><section className="prompt-viewer hud-panel" onMouseDown={e => e.stopPropagation()}><header className="settings-header"><div><div className="eyebrow">IMAGE BUILDER</div><h2>SCENE {String(promptPreview.order).padStart(2, '0')} PROMPTS</h2></div><button className="settings-close" onClick={() => setPromptPreview(null)}>×</button></header><div className="prompt-viewer-body"><div><div className="section-head">IMAGE SIZE</div><div className="scene-image-size prompt-size">{promptPreview.image_width}×{promptPreview.image_height}</div><div className="section-head">POSITIVE PROMPT</div><pre className="prompt-box">{promptPreview.positive_prompt}</pre></div><div><div className="section-head">NEGATIVE PROMPT</div><pre className="prompt-box">{promptPreview.negative_prompt}</pre></div></div></section></div> : null}
    {settingsOpen ? <SettingsOverlay
      settings={state.settings}
      llmApiKeyConfigured={state.llm_api_key_configured}
      onSave={async value => {
        const suppliedKey = Boolean(value.llm_api_key.trim());
        const next = await api.saveSettings(value);
        setState(prev => prev ? {
          ...prev,
          settings: next,
          llm_configured: Boolean(next.llm_base_url && next.llm_model),
          llm_api_key_configured: suppliedKey || prev.llm_api_key_configured,
        } : prev);
      }}
      onClearApiKey={async () => {
        await api.clearLlmApiKey();
        setState(prev => prev ? { ...prev, llm_api_key_configured: false, settings: { ...prev.settings, llm_api_key: '' } } : prev);
      }}
      onError={message => setError(message)}
      onClose={() => setSettingsOpen(false)}
    /> : null}
  </div>;
}
