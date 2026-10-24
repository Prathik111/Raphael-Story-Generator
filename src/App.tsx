import { useEffect, useRef, useState } from 'react';
import { api, subscribeToComfy, subscribeToLlm, subscribeToPipeline } from './tauri';
import type { AppSettings, AppState, Chapter, ComfyGenerationEvent, LlmGenerationEvent, PipelineEvent, RegistryCatalog, RegistryStatusDto, Scene, ServiceStatusBoard, Story, StoryVisualSetup, WebHostStatus } from './types';

const MAX_STYLE_LORAS = 6;

function ReferenceHudBackground() {
  return <iframe className="raphael-bg" src="./reference/index.html" title="Raphael background" aria-hidden="true" />;
}

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

const registryThumbnailCache = new Map<string, string>();
const registryThumbnailRequests = new Map<string, Promise<string | null>>();

async function loadRegistryThumbnail(modelId: string): Promise<string | null> {
  const id = modelId.trim();
  if (!id) return null;

  const cached = registryThumbnailCache.get(id);
  if (cached) return cached;

  const existing = registryThumbnailRequests.get(id);
  if (existing) return existing;

  const request = api.getRegistryModelThumbnails([id])
    .then(thumbnails => {
      const thumbnail = thumbnails[id] || null;
      if (thumbnail) registryThumbnailCache.set(id, thumbnail);
      return thumbnail;
    })
    .catch(() => null)
    .finally(() => {
      registryThumbnailRequests.delete(id);
    });

  registryThumbnailRequests.set(id, request);
  return request;
}

function RegistryModelThumbnail({ modelId }: { modelId: string }) {
  const visualRef = useRef<HTMLDivElement>(null);
  const [thumbnail, setThumbnail] = useState<string | null>(() => registryThumbnailCache.get(modelId) || null);

  useEffect(() => {
    const id = modelId.trim();
    if (!id) return;

    const cached = registryThumbnailCache.get(id);
    if (cached) {
      setThumbnail(cached);
      return;
    }

    const target = visualRef.current;
    if (!target) return;

    let cancelled = false;
    let started = false;

    const load = () => {
      if (started) return;
      started = true;
      void loadRegistryThumbnail(id).then(value => {
        if (!cancelled && value) setThumbnail(value);
      });
    };

    if (typeof IntersectionObserver === 'undefined') {
      load();
      return () => { cancelled = true; };
    }

    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) {
        load();
        observer.disconnect();
      }
    }, {
      root: null,
      rootMargin: '700px 0px',
      threshold: 0.01,
    });

    observer.observe(target);

    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [modelId]);

  return (
    <div ref={visualRef} className="registry-choice-thumbnail" aria-hidden="true">
      {thumbnail ? <img src={thumbnail} alt="" loading="lazy" decoding="async" /> : null}
    </div>
  );
}

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

function RegistryModelChoiceCard({
  model,
  kind,
  selected,
  disabled,
  onClick,
}: {
  model: RegistryCatalog['checkpoints'][number];
  kind: 'checkpoint' | 'lora';
  selected: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  const mark = kind === 'checkpoint' ? 'CKPT' : 'LORA';
  return <button
    type="button"
    className={"registry-choice-card" + (selected ? " selected" : "")}
    onClick={onClick}
    disabled={disabled}
  >
    <div className={"registry-choice-visual " + kind}>
      <RegistryModelThumbnail modelId={model.id} />
      <span>{mark}</span>
      <i />
      <b />
    </div>
    <div className="registry-choice-body">
      <strong title={model.name}>{model.name}</strong>
      <small>{model.base_model || 'BASE UNKNOWN'}</small>
      {model.creator ? <em>{model.creator}</em> : null}
    </div>
    <span className="registry-choice-state" aria-hidden="true">{selected ? '✓' : ''}</span>
  </button>;
}

function VisualSetupOverlay({
  registryStatus,
  catalog,
  catalogError,
  checkpointId,
  setCheckpointId,
  styleLoraIds,
  setStyleLoraIds,
  busy,
  onClose,
}: {
  registryStatus: RegistryStatusDto;
  catalog: RegistryCatalog;
  catalogError: string | null;
  checkpointId: string;
  setCheckpointId: (value: string) => void;
  styleLoraIds: string[];
  setStyleLoraIds: (value: string[]) => void;
  busy: boolean;
  onClose: () => void;
}) {
  const toggleLora = (id: string) => {
    if (styleLoraIds.includes(id)) {
      setStyleLoraIds(styleLoraIds.filter(value => value !== id));
      return;
    }
    if (styleLoraIds.length >= MAX_STYLE_LORAS) return;
    setStyleLoraIds([...styleLoraIds, id]);
  };

  return <div className="model-selection-backdrop" onMouseDown={onClose}>
    <section className="model-selection-panel hud-panel" onMouseDown={event => event.stopPropagation()}>
      <header className="settings-header model-selection-header">
        <div>
          <div className="eyebrow">VISUAL PROFILE</div>
          <h2>MODEL SELECTION</h2>
        </div>
        <button type="button" className="settings-close" onClick={onClose} aria-label="Close model selection">×</button>
      </header>

      <div className="model-selection-scroll">
        {catalogError ? <div className="error-box settings-inline-error">{catalogError}</div> : null}

        <section className="model-selection-section">
          <div className="model-selection-section-head">
            <div>
              <div className="section-head">CHECKPOINT</div>
              <p>Choose the base model for this story.</p>
            </div>
            <span className="selection-count">{checkpointId ? '1 SELECTED' : 'REQUIRED'}</span>
          </div>
          {registryStatus.status !== 'on'
            ? <div className="model-selection-empty">{registryStatus.detail || 'Model Registry is unavailable.'}</div>
            : catalog.checkpoints.length === 0
              ? <div className="model-selection-empty">No checkpoints are registered.</div>
              : <div className="registry-choice-grid">
                  {catalog.checkpoints.map(model => (
                    <RegistryModelChoiceCard
                      key={model.id}
                      model={model}
                      kind="checkpoint"
                      selected={checkpointId === model.id}
                      disabled={busy}
                      onClick={() => setCheckpointId(model.id)}
                    />
                  ))}
                </div>}
        </section>

        <section className="model-selection-section">
          <div className="model-selection-section-head">
            <div>
              <div className="section-head">STYLE LORAS</div>
              <p>Choose up to six style models to keep locked across the story. Compatible character LoRAs can still be added per scene.</p>
            </div>
            <span className="selection-count">{styleLoraIds.length}/{MAX_STYLE_LORAS}</span>
          </div>
          {registryStatus.status !== 'on'
            ? <div className="model-selection-empty">Connect the Model Registry to choose LoRAs.</div>
            : catalog.loras.length === 0
              ? <div className="model-selection-empty">No LoRAs are registered.</div>
              : <div className="registry-choice-grid">
                  {catalog.loras.map(model => (
                    <RegistryModelChoiceCard
                      key={model.id}
                      model={model}
                      kind="lora"
                      selected={styleLoraIds.includes(model.id)}
                      disabled={busy || (!styleLoraIds.includes(model.id) && styleLoraIds.length >= MAX_STYLE_LORAS)}
                      onClick={() => toggleLora(model.id)}
                    />
                  ))}
                </div>}
        </section>
      </div>

      <footer className="settings-footer model-selection-footer">
        <span className="tiny">{styleLoraIds.length}/{MAX_STYLE_LORAS} STYLE LORAS</span>
        <button type="button" className="primary-btn" onClick={onClose}>DONE</button>
      </footer>
    </section>
  </div>;
}

function NewStoryPanel({
  busy,
  prompt,
  setPrompt,
  onGenerate,
  registryStatus,
  catalog,
  checkpointId,
  styleLoraIds,
  onOpenVisualSetup,
}: {
  busy: boolean;
  prompt: string;
  setPrompt: (v: string) => void;
  onGenerate: () => void;
  registryStatus: RegistryStatusDto;
  catalog: RegistryCatalog;
  checkpointId: string;
  styleLoraIds: string[];
  onOpenVisualSetup: () => void;
}) {
  const selectedCheckpoint = catalog.checkpoints.find(model => model.id === checkpointId);
  const selectedLoras = catalog.loras.filter(model => styleLoraIds.includes(model.id));

  return <section className="hero-panel hud-panel">
    <div className="scan-corners" />
    <div className="eyebrow">RAPHAEL STORY ENGINE</div>
    <h1>Turn a prompt into a living story.</h1>
    <p className="hero-copy">Describe the story, then choose the visual models that define its identity. Character and concept/pose LoRAs are selected scene-by-scene.</p>

    <button
      type="button"
      className="visual-setup-trigger"
      onClick={onOpenVisualSetup}
      disabled={busy}
    >
      <span className="visual-setup-trigger-mark" aria-hidden="true"><i /><b /></span>
      <span className="visual-setup-trigger-copy">
        <span className="eyebrow">VISUAL MODELS</span>
        <strong>{selectedCheckpoint?.name || 'SELECT CHECKPOINT'}</strong>
        <small>
          {selectedLoras.length
            ? selectedLoras.map(model => model.name).join(' · ')
            : 'NO STYLE LORAS SELECTED'}
        </small>
      </span>
      <span className="visual-setup-trigger-action">OPEN</span>
    </button>

    {registryStatus.status !== 'on' ? <div className="visual-setup-status">{registryStatus.detail || 'Model Registry is unavailable.'}</div> : null}

    <textarea
      className="story-prompt"
      value={prompt}
      onChange={event => setPrompt(event.target.value)}
      placeholder="Example: A dark fantasy anime about a quiet academy student who discovers that the rival she dislikes is protecting a secret tied to her family..."
      disabled={busy}
      rows={6}
    />
    <div className="hero-actions">
      <button
        className="primary-btn"
        onClick={onGenerate}
        disabled={busy || !prompt.trim() || !checkpointId || registryStatus.status !== 'on'}
      >
        {busy ? 'BUILDING STORY BIBLE…' : 'GENERATE STORY'}
      </button>
    </div>
  </section>;
}

function ServiceHealthPanel({ status }: { status: ServiceStatusBoard }) {
  const items = [status.registry, status.searxng, status.comfyui, status.ollama];
  const label = (value: ServiceStatusBoard['registry']['status']) => value.toUpperCase();
  return <section className="hud-panel inspector-panel service-health-panel">
    <div className="panel-title-row">
      <div><div className="section-head">RAPHAEL SERVICES</div><span className="tiny">LIVE API HEALTH</span></div>
    </div>
    <div className="service-health-list">
      {items.map(item => (
        <div className="service-health-item" key={item.service} title={item.detail || item.url}>
          <span className={'service-health-dot ' + item.status}><i /></span>
          <div className="service-health-copy">
            <strong>{item.service.toUpperCase()}</strong>
            <small>{label(item.status)}</small>
          </div>
        </div>
      ))}
    </div>
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
            {source.content ? <details className="research-source-text">
              <summary><span>VIEW FETCHED TEXT</span><small>{source.content.length.toLocaleString()} CHARS</small></summary>
              <pre>{source.content.slice(0, 3000)}{source.content.length > 3000 ? '\n…[TRUNCATED IN UI]…' : ''}</pre>
            </details> : null}
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


function ChapterView({ story, chapter, onExtractScenes, extracting, onBuildPrompt, onViewPrompt, onQueueImage, onBuildAllPrompts, onQueueAllImages, buildingPrompt, queueing, bulkAction, bulkProgress, operationDisabled, progress, images }: {
  story: Story;
  chapter: Chapter;
  onExtractScenes: () => void;
  extracting: boolean;
  onBuildPrompt: (scene: Scene) => void;
  onViewPrompt: (scene: Scene) => void;
  onQueueImage: (scene: Scene) => void;
  onBuildAllPrompts: () => void;
  onQueueAllImages: () => void;
  buildingPrompt: string | null;
  queueing: string | null;
  bulkAction: 'build_prompts' | 'queue_images' | null;
  bulkProgress: { completed: number; total: number } | null;
  operationDisabled: boolean;
  progress: Record<string, ComfyGenerationEvent>;
  images: Record<string, string>;
}) {
  const hasScenes = chapter.scenes.length > 0;
  return <div className="chapter-view">
    <div className="chapter-header hud-panel">
      <div><div className="eyebrow">CHAPTER {String(chapter.number).padStart(2, '0')}</div><h2>{chapter.title}</h2></div>
      <div className="chapter-header-actions"><span className="status-chip">CANONICAL</span><button className="secondary-btn" onClick={onExtractScenes} disabled={extracting || operationDisabled}>{extracting ? 'EXTRACTING…' : chapter.scenes.length ? 'REBUILD SCENES' : 'EXTRACT SCENES'}</button></div>
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
      <div className="panel-title-row scenes-panel-heading">
        <div><div className="section-head">VISUAL SCENES</div><span className="tiny">ONE IMAGEABLE STORY BEAT PER SCENE</span></div>
        <div className="scene-bulk-actions">
          <span className="status-chip">{chapter.scenes.length} SCENES</span>
          {hasScenes ? <>
            <button className="secondary-btn" onClick={onBuildAllPrompts} disabled={operationDisabled}>{bulkAction === 'build_prompts' ? 'BUILDING ' + (bulkProgress?.completed ?? 0) + '/' + (bulkProgress?.total ?? chapter.scenes.length) : 'BUILD ALL PROMPTS'}</button>
            <button className="primary-btn" onClick={onQueueAllImages} disabled={operationDisabled} title="Queue every scene with a prompt that is not already generated or running">{bulkAction === 'queue_images' ? 'QUEUEING ' + (bulkProgress?.completed ?? 0) + '/' + (bulkProgress?.total ?? chapter.scenes.length) : 'GENERATE ALL IMAGES'}</button>
          </> : null}
        </div>
      </div>
      {bulkAction && bulkProgress ? <div className="bulk-progress-note">{bulkAction === 'build_prompts' ? 'BUILDING SCENE PROMPTS' : 'SUBMITTING IMAGE JOBS'} · {bulkProgress.completed} / {bulkProgress.total} COMPLETE</div> : null}
      {chapter.scenes.length === 0 ? <div className="scene-empty">Extract this chapter into visual beats for the manga/image pipeline.</div> : <div className="scene-grid">{chapter.scenes.map(scene => {
        const canonicalCharacters = scene.characters.map(name => story.bible.characters.find(character =>
          name.trim() === character.id || name.trim().toLowerCase() === character.name.trim().toLowerCase()
        )).filter(Boolean);
        return <article className="scene-card" key={scene.id}>
          <div className="scene-index">SCENE {String(scene.order).padStart(2, '0')}</div><h3>{scene.description}</h3>
          <div className="scene-meta"><span>{scene.location || 'UNKNOWN LOCATION'}</span><span>{scene.time || 'TIME UNSPECIFIED'}</span></div>
          <p className="scene-action">{scene.action}</p>
          {canonicalCharacters.length ? <div className="scene-character-details">
            <div className="section-head">CANONICAL CHARACTER DESIGN</div>
            {canonicalCharacters.map(character => character ? <p key={character.id}><strong>{character.name}</strong><span>Appearance: {character.appearance || 'Not specified'}</span><span>Outfit: {character.clothing || 'Not specified'}</span></p> : null)}
          </div> : scene.characters.length ? <div className="scene-character-details missing">Character names could not be matched to the Story Bible. Prompt building will require canonical names.</div> : null}
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
          <div className="scene-footer"><span className={'scene-status ' + scene.image_status}>{scene.image_status.replace('_', ' ').toUpperCase()}</span><div className="scene-actions"><button className="mini-btn" onClick={() => scene.positive_prompt ? onViewPrompt(scene) : onBuildPrompt(scene)} disabled={operationDisabled || buildingPrompt === scene.id || queueing === scene.id}>{buildingPrompt === scene.id ? 'BUILDING…' : scene.positive_prompt ? 'VIEW PROMPT' : 'BUILD IMAGE PROMPT'}</button>{scene.positive_prompt ? <button className="mini-btn queue-btn" onClick={() => onQueueImage(scene)} disabled={operationDisabled || queueing === scene.id}>{queueing === scene.id ? 'QUEUING…' : scene.image_status === 'queued' || scene.image_status === 'running' ? 'REQUEUE IMAGE' : 'QUEUE IMAGE'}</button> : null}</div></div>
        </article>;
      })}</div>}
    </section>
  </div>;
}

type GalleryPreview = { src: string; title: string; description: string };

function GeneratedImageGallery({ story, images, onOpen, onReload }: {
  story: Story;
  images: Record<string, string>;
  onOpen: (preview: GalleryPreview) => void;
  onReload: (chapterNumber: number, sceneId: string) => void;
}) {
  const generatedScenes = story.chapters
    .flatMap(chapter => chapter.scenes
      .filter(scene => scene.image_status === 'generated')
      .map(scene => ({ chapter, scene })))
    .sort((a, b) => a.chapter.number - b.chapter.number || a.scene.order - b.scene.order);

  return <section className="hud-panel generated-gallery">
    <div className="panel-title-row generated-gallery-heading">
      <div>
        <div className="section-head">GENERATED IMAGE GALLERY</div>
        <span className="tiny">CHAPTER ORDER · SCENE ORDER</span>
      </div>
      <span className="status-chip">{generatedScenes.length} IMAGE{generatedScenes.length === 1 ? '' : 'S'}</span>
    </div>
    {generatedScenes.length === 0
      ? <div className="generated-gallery-empty">No generated images yet. Completed images will appear here in story order.</div>
      : <div className="generated-gallery-grid">
        {generatedScenes.map(({ chapter, scene }) => {
          const src = images[scene.id];
          const label = `CHAPTER ${String(chapter.number).padStart(2, '0')} · SCENE ${String(scene.order).padStart(2, '0')}`;
          return <article className="generated-gallery-item" key={`${chapter.number}-${scene.id}`}>
            <button
              type="button"
              className="generated-gallery-thumb"
              onClick={() => src
                ? onOpen({ src, title: label, description: scene.description })
                : onReload(chapter.number, scene.id)}
              aria-label={src ? `Open ${label}` : `Reload image for ${label}`}
              title={src ? 'Open full-size image' : 'Try loading the saved image again'}
            >
              {src
                ? <img src={src} alt={scene.description} loading="lazy" decoding="async" />
                : <span className="generated-gallery-missing"><strong>IMAGE NOT LOADED</strong><small>Click to try loading again</small></span>}
            </button>
            <div className="generated-gallery-caption">
              <div className="generated-gallery-order">{label}</div>
              <strong>{scene.description}</strong>
              <span>{scene.location || 'Unknown location'} · {scene.time || 'Time unspecified'}</span>
              <small>{scene.image_width} × {scene.image_height}</small>
            </div>
          </article>;
        })}
      </div>}
  </section>;
}

function formatTraceText(value: string, prettyJson: boolean = false): string {
  const normalized = value.replace(/\r\n/g, '\n').replace(/\r/g, '\n').trim();

  if (!normalized) return '';

  if (prettyJson && (normalized.startsWith('{') || normalized.startsWith('['))) {
    try {
      return JSON.stringify(JSON.parse(normalized), null, 2);
    } catch {
      let output = '';
      let indent = 0;
      let inString = false;
      let escaped = false;

      for (const char of normalized) {
        if (escaped) {
          output += char;
          escaped = false;
          continue;
        }

        if (char === '\\' && inString) {
          output += char;
          escaped = true;
          continue;
        }

        if (char === '"') {
          inString = !inString;
          output += char;
          continue;
        }

        if (inString) {
          output += char;
          continue;
        }

        if (char === '{' || char === '[') {
          output = output.trimEnd() + char + '\n';
          indent += 1;
          output += '  '.repeat(indent);
        } else if (char === '}' || char === ']') {
          indent = Math.max(0, indent - 1);
          output = output.trimEnd() + '\n' + '  '.repeat(indent) + char;
        } else if (char === ',') {
          output = output.trimEnd() + ',\n' + '  '.repeat(indent);
        } else if (char === ':') {
          output = output.trimEnd() + ': ';
        } else {
          output += char;
        }
      }

      return output.replace(/\n{3,}/g, '\n\n').trim();
    }
  }

  return normalized
    .replace(/\n[ \t]+/g, '\n')
    .replace(/[ \t]{2,}/g, ' ')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}


type GenerationTrace = {
  generation_id: string;
  stage: string;
  status: LlmGenerationEvent['status'];
  model: string;
  system_prompt: string;
  user_prompt: string;
  thinking: string;
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
              <pre className="trace-box">{formatTraceText(generation.system_prompt)}</pre>
            </div>
            <div className="trace-block">
              <div className="section-head">USER PROMPT</div>
              <pre className="trace-box">{formatTraceText(generation.user_prompt)}</pre>
            </div>
            <div className="trace-block">
              <div className="section-head">MODEL THINKING {(generation.status === 'started' || generation.status === 'token') ? '· STREAMING' : ''}</div>
              <pre className="trace-box thinking">{formatTraceText(generation.thinking || '(no provider thinking stream)')}</pre>
            </div>
            <div className="trace-block">
              <div className="section-head">LLM RESPONSE {(generation.status === 'started' || generation.status === 'token') ? '· STREAMING' : ''}</div>
              <pre className="trace-box response">{formatTraceText(generation.response, true)}{(generation.status === 'started' || generation.status === 'token') ? '▌' : ''}</pre>
            </div>
            {generation.error ? <div className="error-box">{generation.error}</div> : null}
          </div>
        </details>
      ))}
    </div>
  </section>;
}

function ResearchActivityPanel({ pipeline, generations }: {
  pipeline: PipelineEvent[];
  generations: GenerationTrace[];
}) {
  const events = pipeline.filter(event =>
    event.stage.includes('research') || event.stage.startsWith('web_') || event.stage === 'web_gateway'
  );
  const latest = events.slice(-16);
  const liveTrace = generations.slice().reverse().find(trace => trace.stage === 'web_research');
  const latestResearchEvent = latest[latest.length - 1] || null;
  const activeEvent = latestResearchEvent?.status === 'started' ? latestResearchEvent : null;
  const query = liveTrace?.user_prompt.match(/RESEARCH QUERY:\s*([^\r\n]+)/i)?.[1]?.trim();

  return <section className="hud-panel inspector-panel live-research-activity" aria-live="polite">
    <div className="panel-title-row">
      <div>
        <div className="section-head">LIVE RESEARCH TRACE</div>
        <span className="tiny">{activeEvent ? 'WORKING · ' + activeEvent.stage.replaceAll('_', ' ').toUpperCase() : 'SEARCH · TOR FETCH · EVIDENCE CHECK'}</span>
      </div>
      <span className={'trace-live ' + (activeEvent ? 'active' : '')}><i />{activeEvent ? 'LIVE' : 'TRACE'}</span>
    </div>
    {query ? <div className="live-research-query"><span className="section-head">CURRENT QUERY</span><p>{query}</p></div> : null}
    {latest.length
      ? <div className="live-research-log">
          {latest.map(event => <article className={'live-research-event ' + event.status} key={event.event_id}>
            <div><span>{event.stage.replaceAll('_', ' ').toUpperCase()}</span><small>{event.status.toUpperCase()}</small></div>
            <p>{event.message}</p>
          </article>)}
        </div>
      : <div className="registry-empty">No research activity in this session yet. Use a prompt naming an external character, series, world, or topic, then start story generation.</div>}
    {liveTrace ? <details className="live-research-raw">
      <summary><span className="section-head">EXTRACTOR LLM RESPONSE</span><small>{liveTrace.status.toUpperCase()}</small></summary>
      <pre className="trace-box response">{formatTraceText(liveTrace.response || '(waiting for the extractor response)', true)}</pre>
      {liveTrace.error ? <div className="error-box">{liveTrace.error}</div> : null}
    </details> : null}
    <p className="settings-hint live-research-footnote">Search and source events remain visible while story creation is in progress. Validated source cards and evidence appear in the story research panel after creation finishes.</p>
  </section>;
}

function LanWebHostControl() {
  const [host, setHost] = useState<WebHostStatus>({
    running: false, port: 1425, lan_url: null, detail: 'Checking LAN host status…',
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let disposed = false;
    void api.getWebHostStatus()
      .then(status => { if (!disposed) setHost(status); })
      .catch(reason => { if (!disposed) setError(toErrorMessage(reason)); });
    return () => { disposed = true; };
  }, []);

  const toggleHost = async () => {
    setBusy(true);
    setError(null);
    setCopied(false);
    try {
      const status = host.running ? await api.stopWebHost() : await api.startWebHost();
      setHost(status);
    } catch (reason) {
      setError(toErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const copyAddress = async () => {
    if (!host.lan_url) return;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(host.lan_url);
      } else {
        const input = document.createElement('textarea');
        input.value = host.lan_url;
        input.style.position = 'fixed';
        input.style.opacity = '0';
        document.body.appendChild(input);
        input.focus();
        input.select();
        const success = document.execCommand('copy');
        input.remove();
        if (!success) throw new Error('Copy is unavailable in this browser.');
      }
      setCopied(true);
    } catch (reason) {
      setError(toErrorMessage(reason));
    }
  };

  return <div className="lan-web-host">
    <button
      type="button"
      className={'lan-web-host-toggle' + (host.running ? ' active' : '')}
      onClick={() => void toggleHost()}
      disabled={busy}
      title={host.running ? 'Stop LAN web hosting' : 'Share Raphael with devices on this LAN'}
    >
      <span className="lan-web-host-dot" />
      {busy ? (host.running ? 'STOPPING…' : 'STARTING…') : host.running ? 'WEB HOST ON' : 'WEB HOST'}
    </button>
    {host.running && host.lan_url ? <div className="lan-web-host-popover">
      <div className="eyebrow">RAPHAEL LAN ADDRESS</div>
      <code>{host.lan_url}</code>
      <button type="button" className="mini-btn" onClick={() => void copyAddress()}>{copied ? 'COPIED' : 'COPY ADDRESS'}</button>
      <small>Open this address on another device connected to the same trusted LAN. No Ollama API port is exposed separately.</small>
    </div> : null}
    {error ? <div className="lan-web-host-error" role="alert">{error}</div> : null}
  </div>;
}

function SettingsOverlay({ settings, catalog, registryStatus, catalogError, llmApiKeyConfigured, onSave, onClearApiKey, onClose, onError }: { settings: AppSettings; catalog: RegistryCatalog; registryStatus: RegistryStatusDto; catalogError: string | null; llmApiKeyConfigured: boolean; onSave: (settings: AppSettings) => Promise<void>; onClearApiKey: () => Promise<void>; onClose: () => void; onError: (message: string) => void }) {
  const [draft, setDraft] = useState(settings);
  const [busy, setBusy] = useState(false);
  const [researchTesting, setResearchTesting] = useState(false);
  const [researchTestResult, setResearchTestResult] = useState<string | null>(null);
  const [ollamaDetecting, setOllamaDetecting] = useState(false);
  const [ollamaDetectionResult, setOllamaDetectionResult] = useState<string | null>(null);
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [styleLoraSearch, setStyleLoraSearch] = useState('');
  const visibleStyleLoras = catalog.loras.filter(model =>
    [model.name, model.base_model || '', model.creator || '']
      .some(value => value.toLowerCase().includes(styleLoraSearch.trim().toLowerCase()))
  );

  const toggleDefaultStyleLora = (id: string) => {
    const selected = draft.default_style_lora_ids || [];
    if (selected.includes(id)) {
      setDraft({ ...draft, default_style_lora_ids: selected.filter(value => value !== id) });
      return;
    }
    if (selected.length >= MAX_STYLE_LORAS) return;
    setDraft({ ...draft, default_style_lora_ids: [...selected, id] });
  };

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
      setResearchTestResult(toErrorMessage(error));
    } finally {
      setResearchTesting(false);
    }
  };

  const detectOllamaModels = async () => {
    setOllamaDetecting(true);
    setOllamaDetectionResult(null);
    try {
      const detected = await api.detectOllamaModels(draft.llm_base_url);
      const selected = detected.models.includes(draft.llm_model)
        ? draft.llm_model
        : detected.models[0];
      setOllamaModels(detected.models);
      setDraft(current => ({
        ...current,
        llm_base_url: detected.openai_base_url,
        llm_model: selected || current.llm_model,
      }));
      setOllamaDetectionResult(`Loaded ${detected.models.length} model(s) from this host. Selected ${selected || detected.models[0]}.`);
    } catch (error) {
      setOllamaDetectionResult(toErrorMessage(error));
    } finally {
      setOllamaDetecting(false);
    }
  };

  useEffect(() => {
    // A browser client is already talking to the host-side API; populate the
    // model selector from that PC's local Ollama as soon as Settings opens.
    if (api.isWebApp) void detectOllamaModels();
  }, []);

  return <div className="settings-backdrop" onMouseDown={onClose}><section className="settings-panel hud-panel" onMouseDown={e => e.stopPropagation()}>
    <header className="settings-header"><div><div className="eyebrow">RAPHAEL CORE</div><h2>ENGINE SETTINGS</h2></div><button className="settings-close" onClick={onClose}>×</button></header>
    <div className="settings-scroll">
      <div className="section-head">OPENAI-COMPATIBLE LLM</div>
      <label>Base URL<input value={draft.llm_base_url} onChange={e => setDraft({ ...draft, llm_base_url: e.target.value })} placeholder="http://127.0.0.1:11434/v1"/></label>
      <label>Model{ollamaModels.length ? <select value={draft.llm_model} onChange={e => setDraft({ ...draft, llm_model: e.target.value })}>{ollamaModels.map(model => <option value={model} key={model}>{model}</option>)}</select> : <input value={draft.llm_model} onChange={e => setDraft({ ...draft, llm_model: e.target.value })} placeholder="qwen3:8b"/>}</label>
      <label>API key<input type="password" value={draft.llm_api_key} onChange={e => setDraft({ ...draft, llm_api_key: e.target.value })} placeholder={llmApiKeyConfigured ? 'Stored securely · leave blank to keep current key' : 'Optional for local endpoints'}/>{llmApiKeyConfigured ? <button type="button" className="mini-btn settings-clear-key" onClick={() => void onClearApiKey()}>CLEAR STORED KEY</button> : null}</label>

      <div className="section-head setting-gap">OLLAMA AUTO-DETECTION</div>
      <small className="settings-hint">Loads the models installed on the computer running Raphael. When you open Raphael from another device on the LAN, this dropdown still lists the host's models. Ollama stays on the host; only the Raphael web app is shared.</small>
      <div className="ollama-action-row">
        <button type="button" className="secondary-btn" onClick={() => void detectOllamaModels()} disabled={ollamaDetecting}>{ollamaDetecting ? 'LOADING…' : 'LOAD HOST MODELS'}</button>
        {ollamaModels.length ? <span className="tiny">{ollamaModels.length} MODEL(S)</span> : null}
      </div>
      {ollamaModels.length ? <label>Host model<select value={draft.llm_model} onChange={e => setDraft({ ...draft, llm_model: e.target.value })}>{ollamaModels.map(model => <option value={model} key={model}>{model}</option>)}</select></label> : null}
      {ollamaDetectionResult ? <div className={ollamaDetectionResult.startsWith('Loaded ') ? 'ollama-inline-result' : 'error-box'}>{ollamaDetectionResult}</div> : null}

      <div className="section-head setting-gap">VISUAL MODEL DEFAULTS · STYLE LORAS</div>
      <div className="settings-lora-toolbar">
        <span className="selection-count">{(draft.default_style_lora_ids || []).length}/{MAX_STYLE_LORAS} SELECTED</span>
        <button type="button" className="mini-btn" onClick={() => setDraft({ ...draft, default_style_lora_ids: [] })} disabled={!draft.default_style_lora_ids?.length}>CLEAR SELECTION</button>
      </div>
      <p className="settings-hint settings-lora-help">These LoRAs are locked into every new story and applied across its scenes. Raphael will still search the Registry for compatible character-identity LoRAs that match characters in each scene, and add those automatically. A manually selected LoRA will not be added twice. Choose a compatible checkpoint in VISUAL MODELS before creating the story.</p>
      <label className="settings-lora-search">FILTER AVAILABLE LORAS<input type="search" value={styleLoraSearch} onChange={e => setStyleLoraSearch(e.target.value)} placeholder="Search by LoRA name, base model, or creator"/></label>
      {registryStatus.status !== 'on'
        ? <div className="model-selection-empty">{catalogError || registryStatus.detail || 'Connect the Model Registry to select LoRAs.'}</div>
        : catalog.loras.length === 0
          ? <div className="model-selection-empty">No LoRAs are registered in the Model Registry.</div>
          : visibleStyleLoras.length === 0
            ? <div className="model-selection-empty">No LoRAs match “{styleLoraSearch}”.</div>
            : <div className="settings-lora-grid">
              {visibleStyleLoras.map(model => (
                <RegistryModelChoiceCard
                  key={model.id}
                  model={model}
                  kind="lora"
                  selected={(draft.default_style_lora_ids || []).includes(model.id)}
                  disabled={!(draft.default_style_lora_ids || []).includes(model.id) && (draft.default_style_lora_ids || []).length >= MAX_STYLE_LORAS}
                  onClick={() => toggleDefaultStyleLora(model.id)}
                />
              ))}
            </div>}

      <div className="section-head setting-gap">LAN WEB HOSTING</div>
      <label>Web host port<input type="number" min="1024" max="65535" step="1" value={draft.web_host_port} onChange={e => setDraft({ ...draft, web_host_port: Number(e.target.value) || 1425 })}/></label>
      <small className="settings-hint">Choose the TCP port used for the Raphael LAN web address (1024–65535). Save settings, then stop and restart WEB HOST from the top bar if it is already running. The new port takes effect when hosting starts.</small>

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
      <div className="research-test-row"><button type="button" className="secondary-btn" onClick={() => void testPrivateResearch()} disabled={researchTesting}>{researchTesting ? 'TESTING…' : 'TEST FULL RESEARCH PIPELINE'}</button>{researchTestResult ? <span className="tiny">{researchTestResult}</span> : null}</div>

      <div className="section-head setting-gap">COMFYUI</div>
      <label>API URL<input value={draft.comfyui_url} onChange={e => setDraft({ ...draft, comfyui_url: e.target.value })} placeholder="http://127.0.0.1:8188"/></label>
      <div className="section-head setting-gap">FIXED IMAGE SIZE</div>
      <div className="settings-grid-two">
        <label>Width (px)<input type="number" min={512} max={4096} step={8} value={draft.image_width} onChange={e => setDraft({ ...draft, image_width: Number(e.target.value) || 1920 })}/></label>
        <label>Height (px)<input type="number" min={512} max={4096} step={8} value={draft.image_height} onChange={e => setDraft({ ...draft, image_height: Number(e.target.value) || 1080 })}/></label>
      </div>
      <small className="settings-hint">Every scene uses the same configured dimensions, including existing scenes when you build prompts or queue images. Default: 1920 × 1080. Each value must be between 512 and 4096 and divisible by 8.</small>
      <label>API workflow template (optional)<small className="settings-hint">Leave blank to use Raphael Image Generator's built-in graph (UNETLoader or CheckpointLoaderSimple, Anima CLIP/VAE, model-only LoRA chain, EmptyLatentImage, KSampler and SaveImage). This built-in graph targets the same Anima setup as Image Generator. For a different model architecture, paste a ComfyUI API-format template with POSITIVE_PROMPT, NEGATIVE_PROMPT, SEED, STORY_ID, SCENE_ID, CHECKPOINT, and IMAGE_WIDTH/IMAGE_HEIGHT (or IMAGE_SIZE). A custom workflow must expose an image-size injection point and a CheckpointLoaderSimple or CheckpointLoader node; Raphael inserts LoRA loaders into that graph.</small><textarea className="workflow-input" value={draft.comfyui_workflow_json} onChange={e => setDraft({ ...draft, comfyui_workflow_json: e.target.value })} placeholder='Optional: paste a custom ComfyUI API workflow JSON template...'/></label>
    </div>
    {saveError ? <div className="settings-inline-error error-box">{saveError}</div> : null}
    <footer className="settings-footer"><button className="secondary-btn" onClick={onClose}>CANCEL</button><button className="primary-btn" onClick={() => void save()} disabled={busy}>{busy ? 'SAVING…' : 'SAVE SETTINGS'}</button></footer>
  </section></div>;
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null); const [story, setStory] = useState<Story | null>(null);
  const [prompt, setPrompt] = useState(''); const [directive, setDirective] = useState('');
  const [busy, setBusy] = useState(false); const [extracting, setExtracting] = useState(false); const [buildingPrompt, setBuildingPrompt] = useState<string | null>(null); const [queueing, setQueueing] = useState<string | null>(null);
  const [bulkAction, setBulkAction] = useState<'build_prompts' | 'queue_images' | null>(null);
  const [bulkProgress, setBulkProgress] = useState<{ completed: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null); const [settingsOpen, setSettingsOpen] = useState(false); const [visualSetupOpen, setVisualSetupOpen] = useState(false); const [promptPreview, setPromptPreview] = useState<Scene | null>(null);
  const [registryStatus, setRegistryStatus] = useState<RegistryStatusDto>({ status: 'starting', url: 'http://127.0.0.1:43217', detail: 'Starting Raphael Model Registry…' });
  const [registryCatalog, setRegistryCatalog] = useState<RegistryCatalog>({ checkpoints: [], checkpoint_total: 0, loras: [], lora_total: 0 });
  const [registryCatalogError, setRegistryCatalogError] = useState<string | null>(null);
  const [checkpointId, setCheckpointId] = useState('');
  const [styleLoraIds, setStyleLoraIds] = useState<string[]>([]);
  const [generationTraces, setGenerationTraces] = useState<GenerationTrace[]>([]);
  const [pipelineTrace, setPipelineTrace] = useState<PipelineEvent[]>([]);
  const [comfyProgress, setComfyProgress] = useState<Record<string, ComfyGenerationEvent>>({});
  const [sceneImages, setSceneImages] = useState<Record<string, string>>({});
  const [galleryPreview, setGalleryPreview] = useState<GalleryPreview | null>(null);
  const [selectedChapterNumber, setSelectedChapterNumber] = useState<number | null>(null);
  const [serviceStatus, setServiceStatus] = useState<ServiceStatusBoard>({
    registry: { service: 'registry', status: 'checking', url: 'http://127.0.0.1:43217', detail: 'Checking Registry API…' },
    searxng: { service: 'searxng', status: 'checking', url: 'http://127.0.0.1:8080', detail: 'Checking SearXNG API…' },
    comfyui: { service: 'comfyui', status: 'checking', url: 'http://127.0.0.1:8188', detail: 'Checking ComfyUI API…' },
    ollama: { service: 'ollama', status: 'checking', url: 'http://127.0.0.1:11434', detail: 'Checking Ollama API…' },
  });
  const serviceProbeActive = useRef(false);
  const initialSettingsDefaultsApplied = useRef(false);

  const loadState = async () => {
    const next = await api.getState();
    setState(next);
    if (!initialSettingsDefaultsApplied.current) {
      initialSettingsDefaultsApplied.current = true;
      setStyleLoraIds((next.settings.default_style_lora_ids || []).slice(0, MAX_STYLE_LORAS));
    }
    return next;
  };
  const selectStory = async (id: string) => {
    setError(null);
    try {
      setDirective('');
      setPromptPreview(null);
      setSceneImages({});
      setGalleryPreview(null);
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
            thinking: event.thinking || '',
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
            thinking: event.thinking ?? (event.thinking_delta ? item.thinking + event.thinking_delta : item.thinking),
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
          .catch(error => setError(`ComfyUI image load failed: ${toErrorMessage(error)}`));
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
            detail: toErrorMessage(error),
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
    if (!story) return;
    let disposed = false;
    const generatedScenes = story.chapters.flatMap(chapter =>
      chapter.scenes
        .filter(scene => scene.image_status === 'generated')
        .map(scene => ({ chapter, scene }))
    );

    void Promise.all(generatedScenes.map(async ({ chapter, scene }) => {
      try {
        const image = await api.getSceneImage(story.id, chapter.number, scene.id);
        if (!disposed && image) {
          setSceneImages(current => current[scene.id] ? current : { ...current, [scene.id]: image });
        }
      } catch {
        // A missing or deleted image should not block the rest of the gallery.
      }
    }));
    return () => { disposed = true; };
  }, [story?.id]);

  const refreshServiceStatus = async () => {
    if (serviceProbeActive.current) return;
    serviceProbeActive.current = true;
    try {
      setServiceStatus(await api.getServiceStatus());
    } catch (e) {
      const message = 'Health probe failed: ' + toErrorMessage(e);
      setServiceStatus(current => ({
        registry: { ...current.registry, status: 'checking', detail: message },
        searxng: { ...current.searxng, status: 'checking', detail: message },
        comfyui: { ...current.comfyui, status: 'checking', detail: message },
        ollama: { ...current.ollama, status: 'checking', detail: message },
      }));
    } finally {
      serviceProbeActive.current = false;
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
        setRegistryCatalogError(toErrorMessage(error));
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

  const appendLocalPipelineEvent = (stage: string, status: PipelineEvent['status'], message: string) => {
    const eventId = `ui-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setPipelineTrace(current => [...current, {
      event_id: eventId,
      stage,
      status,
      message,
    }]);
    return eventId;
  };

  const updateLocalPipelineEvent = (eventId: string, status: PipelineEvent['status'], message: string) => {
    setPipelineTrace(current => current.map(event => event.event_id === eventId ? { ...event, status, message } : event));
  };

  const generateStory = async () => {
    if (!prompt.trim() || busy || !checkpointId || registryStatus.status !== 'on') return;
    setBusy(true);
    setError(null);
    const traceId = appendLocalPipelineEvent('generation', 'started', 'Generation started — preparing research and model calls…');
    try {
      const visualSetup: StoryVisualSetup = { checkpoint_id: checkpointId, style_lora_ids: styleLoraIds.slice(0, MAX_STYLE_LORAS) };
      const next = await api.createStory(prompt.trim(), visualSetup);
      setStory(next);
      setSelectedChapterNumber(next.chapters[next.chapters.length - 1]?.number ?? null);
      setPrompt('');
      await loadState();
      updateLocalPipelineEvent(traceId, 'completed', 'Generation completed successfully.');
    } catch (e) {
      const message = toErrorMessage(e);
      updateLocalPipelineEvent(traceId, 'error', message);
      setError(message);
    } finally {
      setBusy(false);
    }
  };

  const generateNext = async () => {
    if (!story || busy) return;
    setBusy(true);
    setError(null);
    const traceId = appendLocalPipelineEvent('generation', 'started', `Chapter generation started — preparing Chapter ${story.chapters.length + 1}…`);
    try {
      const next = await api.generateNextChapter(story.id, directive.trim());
      setStory(next);
      setSelectedChapterNumber(next.chapters[next.chapters.length - 1]?.number ?? null);
      setDirective('');
      await loadState();
      updateLocalPipelineEvent(traceId, 'completed', 'Chapter generation completed successfully.');
    } catch (e) {
      const message = toErrorMessage(e);
      updateLocalPipelineEvent(traceId, 'error', message);
      setError(message);
    } finally {
      setBusy(false);
    }
  };
  const extractScenes = async (chapter: Chapter) => {
    if (!story || extracting || bulkAction) return;
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

  const buildPrompt = async (scene: Scene) => {
    if (!story || buildingPrompt || bulkAction || queueing) return;
    const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id));
    if (!chapter) return;
    setBuildingPrompt(scene.id);
    setError(null);
    try {
      setStory(await api.buildScenePrompt(story.id, chapter.number, scene.id));
      await loadState();
    } catch (e) {
      setError(toErrorMessage(e));
    } finally {
      setBuildingPrompt(null);
    }
  };

  const buildAllPrompts = async (chapter: Chapter) => {
    if (!story || bulkAction || buildingPrompt || queueing || extracting || chapter.scenes.length === 0) return;
    const existingPrompts = chapter.scenes.filter(scene => scene.positive_prompt.trim()).length;
    if (existingPrompts > 0 && !window.confirm('Build prompts for all ' + chapter.scenes.length + ' scenes? ' + existingPrompts + ' existing prompt(s) will be replaced; images will not be regenerated.')) return;

    setBulkAction('build_prompts');
    setBulkProgress({ completed: 0, total: chapter.scenes.length });
    setError(null);
    const failures: string[] = [];
    for (const [index, scene] of chapter.scenes.entries()) {
      try {
        const updatedStory = await api.buildScenePrompt(story.id, chapter.number, scene.id);
        setStory(updatedStory);
      } catch (e) {
        failures.push('Scene ' + String(scene.order).padStart(2, '0') + ': ' + toErrorMessage(e));
      }
      setBulkProgress({ completed: index + 1, total: chapter.scenes.length });
    }

    try {
      setStory(await api.getStory(story.id));
      await loadState();
    } catch (e) {
      failures.push('Refresh: ' + toErrorMessage(e));
    }

    setError(failures.length
      ? 'Prompt building finished with ' + failures.length + ' error(s): ' + failures.slice(0, 3).join(' · ') + (failures.length > 3 ? ' · …' : '')
      : null);
    setBulkAction(null);
    setBulkProgress(null);
  };

  const viewPrompt = (scene: Scene) => setPromptPreview(scene);

  const queueImage = async (scene: Scene) => {
    if (!story || queueing || bulkAction || buildingPrompt) return;
    const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id));
    if (!chapter) return;
    setQueueing(scene.id);
    setError(null);
    try {
      setStory(await api.queueSceneImage(story.id, chapter.number, scene.id));
      await loadState();
    } catch (e) {
      setError(toErrorMessage(e));
    } finally {
      setQueueing(null);
    }
  };

  const generateAllImages = async (chapter: Chapter) => {
    if (!story || bulkAction || buildingPrompt || queueing || extracting || chapter.scenes.length === 0) return;
    const missingPrompts = chapter.scenes.filter(scene => !scene.positive_prompt.trim());
    if (missingPrompts.length) {
      setError('Build all prompts first. ' + missingPrompts.length + ' scene(s) still need an image prompt.');
      return;
    }
    const targets = chapter.scenes.filter(scene => !['generated', 'queued', 'running'].includes(scene.image_status));
    if (!targets.length) {
      setError('All scene images are already generated or queued/running. Individual scenes can still be requeued.');
      return;
    }

    setBulkAction('queue_images');
    setBulkProgress({ completed: 0, total: targets.length });
    setError(null);
    const failures: string[] = [];
    for (const [index, scene] of targets.entries()) {
      try {
        const updatedStory = await api.queueSceneImage(story.id, chapter.number, scene.id);
        setStory(updatedStory);
      } catch (e) {
        failures.push('Scene ' + String(scene.order).padStart(2, '0') + ': ' + toErrorMessage(e));
      }
      setBulkProgress({ completed: index + 1, total: targets.length });
    }

    try {
      setStory(await api.getStory(story.id));
      await loadState();
    } catch (e) {
      failures.push('Refresh: ' + toErrorMessage(e));
    }
    setError(failures.length
      ? 'Image queueing finished with ' + failures.length + ' error(s): ' + failures.slice(0, 3).join(' · ') + (failures.length > 3 ? ' · …' : '')
      : null);
    setBulkAction(null);
    setBulkProgress(null);
  };

  const currentChapter = story
    ? story.chapters.find(chapter => chapter.number === selectedChapterNumber) || story.chapters[story.chapters.length - 1] || null
    : null;
  const researchTrace = generationTraces.slice().reverse().find(trace => trace.stage.includes('research')) || null;

  if (!state) return (
    <div className="setup-shell">
      <ReferenceHudBackground />
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
    <ReferenceHudBackground />
    <div className="noise" />
    <BackgroundRaphael />
    <header className="topbar">
      <div className="brand"><PulseMark/><div><div className="brand-title">RAPHAEL</div><div className="brand-subtitle">STORY GENERATOR</div></div></div>
      {!api.isWebApp ? <LanWebHostControl /> : null}
    </header>

    <div className="workspace">
      <aside className="sidebar">
        <div className="sidebar-head"><div><div className="eyebrow">LIBRARY</div><h2>STORIES</h2></div><button className="square-btn" disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing) || bulkAction !== null} onClick={() => { setStory(null); setPrompt(''); setDirective(''); setPromptPreview(null); }}>+</button></div>
        <div className="story-list">{state.stories.length === 0 ? <div className="empty-sidebar">No stories yet.<br/>Create the first one from the story prompt.</div> : state.stories.map(item => <StoryCard key={item.id} story={item} selected={story?.id === item.id} disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing) || bulkAction !== null} onClick={() => void selectStory(item.id)}/>)}</div>
        <ServiceHealthPanel status={serviceStatus}/>
        <div className="sidebar-foot">
          <button type="button" className="settings-trigger" aria-label="Open settings" title="SETTINGS" onClick={() => setSettingsOpen(true)}>
            <span aria-hidden="true">⚙</span><b>SETTINGS</b>
          </button>
        </div>
      </aside>

      <main className="main-panel">
        {error ? <div className="error-banner error-box" role="alert"><strong>PIPELINE ERROR</strong><span>{error}</span><button type="button" className="mini-btn" onClick={() => setError(null)}>DISMISS</button></div> : null}
        {!story ? <NewStoryPanel busy={busy} prompt={prompt} setPrompt={setPrompt} onGenerate={() => void generateStory()} registryStatus={registryStatus} catalog={registryCatalog} checkpointId={checkpointId} styleLoraIds={styleLoraIds} onOpenVisualSetup={() => setVisualSetupOpen(true)}/> : <>
          <section className="story-header hud-panel">
            <div><div className="eyebrow">STORY BIBLE · {story.id.slice(0, 8).toUpperCase()}</div><h1>{story.title}</h1><p>{story.bible.premise}</p><label className="chapter-selector">VIEW CHAPTER<select value={selectedChapterNumber ?? story.chapters[story.chapters.length - 1]?.number ?? 1} onChange={e => setSelectedChapterNumber(Number(e.target.value))}>{story.chapters.map(chapter => <option key={chapter.number} value={chapter.number}>CHAPTER {chapter.number} · {chapter.title}</option>)}</select></label></div>
            <div className="story-header-tags"><TagRow values={story.metadata.tone} tone="tone"/></div>
          </section>
          <IntroductionView story={story}/>
          <GeneratedImageGallery
            story={story}
            images={sceneImages}
            onOpen={setGalleryPreview}
            onReload={(chapterNumber, sceneId) => {
              void api.getSceneImage(story.id, chapterNumber, sceneId)
                .then(image => {
                  if (image) setSceneImages(current => ({ ...current, [sceneId]: image }));
                  else setError('The generated image file is missing from Raphael’s local image storage.');
                })
                .catch(loadError => setError(`Image reload failed: ${toErrorMessage(loadError)}`));
            }}
          />
          {currentChapter ? <ChapterView story={story} chapter={currentChapter} onExtractScenes={() => void extractScenes(currentChapter)} extracting={extracting} onBuildPrompt={scene => void buildPrompt(scene)} onViewPrompt={viewPrompt} onQueueImage={queueImage} onBuildAllPrompts={() => void buildAllPrompts(currentChapter)} onQueueAllImages={() => void generateAllImages(currentChapter)} buildingPrompt={buildingPrompt} queueing={queueing} bulkAction={bulkAction} bulkProgress={bulkProgress} operationDisabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing) || bulkAction !== null} progress={comfyProgress} images={sceneImages}/> : null}
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
        <GenerationMonitor
          generations={generationTraces}
          pipeline={pipelineTrace}
          onClear={() => { setGenerationTraces([]); setPipelineTrace([]); }}
        />
        <ResearchActivityPanel pipeline={pipelineTrace} generations={generationTraces}/>
        {story ? <ResearchPanel story={story} researchTrace={researchTrace}/> : null}
        {story ? <MetadataPanel story={story}/> : null}
      </aside>
    </div>

    {visualSetupOpen ? <VisualSetupOverlay
      registryStatus={registryStatus}
      catalog={registryCatalog}
      catalogError={registryCatalogError}
      checkpointId={checkpointId}
      setCheckpointId={setCheckpointId}
      styleLoraIds={styleLoraIds}
      setStyleLoraIds={setStyleLoraIds}
      busy={busy}
      onClose={() => setVisualSetupOpen(false)}
    /> : null}
    {galleryPreview ? <div className="overlay gallery-preview-overlay" onMouseDown={() => setGalleryPreview(null)}>
      <section className="gallery-preview-dialog hud-panel" role="dialog" aria-modal="true" aria-label={galleryPreview.title} onMouseDown={e => e.stopPropagation()}>
        <header className="settings-header">
          <div><div className="eyebrow">GENERATED IMAGE</div><h2>{galleryPreview.title}</h2></div>
          <button type="button" className="settings-close" aria-label="Close image preview" onClick={() => setGalleryPreview(null)}>×</button>
        </header>
        <div className="gallery-preview-image-wrap"><img src={galleryPreview.src} alt={galleryPreview.description} /></div>
        <p className="gallery-preview-description">{galleryPreview.description}</p>
      </section>
    </div> : null}
    {promptPreview ? <div className="overlay" onMouseDown={() => setPromptPreview(null)}><section className="prompt-viewer hud-panel" onMouseDown={e => e.stopPropagation()}><header className="settings-header"><div><div className="eyebrow">IMAGE BUILDER</div><h2>SCENE {String(promptPreview.order).padStart(2, '0')} PROMPTS</h2></div><button className="settings-close" onClick={() => setPromptPreview(null)}>×</button></header><div className="prompt-viewer-body"><div><div className="section-head">IMAGE SIZE</div><div className="scene-image-size prompt-size">{promptPreview.image_width}×{promptPreview.image_height}</div><div className="section-head">POSITIVE PROMPT</div><pre className="prompt-box">{promptPreview.positive_prompt}</pre></div><div><div className="section-head">NEGATIVE PROMPT</div><pre className="prompt-box">{promptPreview.negative_prompt}</pre></div></div></section></div> : null}
    {settingsOpen ? <SettingsOverlay
      settings={state.settings}
      catalog={registryCatalog}
      registryStatus={registryStatus}
      catalogError={registryCatalogError}
      llmApiKeyConfigured={state.llm_api_key_configured}
      onSave={async value => {
        const suppliedKey = Boolean(value.llm_api_key.trim());
        const next = await api.saveSettings(value);
        setStyleLoraIds((next.default_style_lora_ids || []).slice(0, MAX_STYLE_LORAS));
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