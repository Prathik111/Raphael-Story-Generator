import { useEffect, useMemo, useState } from 'react';
import { api } from './tauri';
import type { AppSettings, AppState, Chapter, RegistryCatalog, RegistryStatusDto, Scene, Story } from './types';

function PulseMark() {
  return <div className="raphael-core" aria-label="Raphael"><span className="core-dot"/><i className="core-orbit orbit-a"/><i className="core-orbit orbit-b"/><i className="core-orbit orbit-c"/></div>;
}

function splitTags(values: string[]) { return values.filter(Boolean).slice(0, 12); }

function formatDate(value: string) {
  const unix = value.match(/^unix:(\\d+)$/);
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

function NewStoryPanel({ busy, prompt, setPrompt, onGenerate }: { busy: boolean; prompt: string; setPrompt: (v: string) => void; onGenerate: () => void }) {
  return <section className="hero-panel hud-panel">
    <div className="scan-corners" />
    <div className="eyebrow">RAPHAEL STORY ENGINE · STORY BIBLE FIRST</div>
    <h1>Turn a prompt into a living story.</h1>
    <p className="hero-copy">Describe the story, characters, world and mood you have in mind. Raphael extracts the canon, generates the introduction and Chapter 1, then keeps the story state for every continuation.</p>
    <textarea className="story-prompt" value={prompt} onChange={event => setPrompt(event.target.value)} placeholder="Example: A dark fantasy anime about a quiet 17-year-old academy student who discovers that the rival she dislikes is protecting a secret tied to her family..." disabled={busy} rows={6}/>
    <div className="hero-actions"><button className="primary-btn" onClick={onGenerate} disabled={busy || !prompt.trim()}>{busy ? 'BUILDING STORY BIBLE…' : 'GENERATE STORY'}</button><span className="tiny">INTRODUCTION + CHAPTER 1 · CANONICAL STATE SAVED</span></div>
  </section>;
}


function RegistryPanel({ status, catalog, error }: { status: RegistryStatusDto; catalog: RegistryCatalog; error: string | null }) {
  const statusLabel = status.status === 'on' ? 'ON' : status.status === 'starting' ? 'STARTING' : 'OFF';
  const renderModelList = (models: RegistryCatalog['checkpoints']) => models.slice(0, 6).map(model => (
    <div className="registry-model" key={model.id} title={model.base_model ? \`\${model.name} · \${model.base_model}\` : model.name}>
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
      <span className={\`registry-state \${status.status}\`}><i />{statusLabel}</span>
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

function ChapterView({ story, chapter, onExtractScenes, extracting, onBuildPrompt, onViewPrompt, onQueueImage, buildingPrompt, queueing }: { story: Story; chapter: Chapter; onExtractScenes: () => void; extracting: boolean; onBuildPrompt: (scene: Scene) => void; onViewPrompt: (scene: Scene) => void; onQueueImage: (scene: Scene) => void; buildingPrompt: string | null; queueing: string | null; }) {
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
        <div className="scene-footer"><span className={`scene-status ${scene.image_status}`}>{scene.image_status.replace('_', ' ').toUpperCase()}</span><div className="scene-actions"><button className="text-btn" onClick={() => scene.positive_prompt ? onViewPrompt(scene) : onBuildPrompt(scene)} disabled={buildingPrompt === scene.id || queueing === scene.id}>{buildingPrompt === scene.id ? 'BUILDING…' : scene.positive_prompt ? 'VIEW PROMPT' : 'BUILD IMAGE PROMPT'}</button>{scene.positive_prompt ? <button className="text-btn queue-btn" onClick={() => onQueueImage(scene)} disabled={queueing === scene.id}>{queueing === scene.id ? 'QUEUING…' : scene.image_status === 'queued' ? 'REQUEUE IMAGE' : 'QUEUE IMAGE'}</button> : null}</div></div>
      </article>)}</div>}
    </section>
  </div>;
}

function SettingsOverlay({ settings, onSave, onClose, onError }: { settings: AppSettings; onSave: (settings: AppSettings) => Promise<void>; onClose: () => void; onError: (message: string) => void }) {
  const [draft, setDraft] = useState(settings); const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await onSave(draft);
      onClose();
    } catch (error) {
      onError(String(error));
      onClose();
    } finally {
      setBusy(false);
    }
  };
  return <div className="overlay" onMouseDown={onClose}><section className="settings-panel hud-panel" onMouseDown={e => e.stopPropagation()}>
    <header className="settings-header"><div><div className="eyebrow">RAPHAEL CORE</div><h2>ENGINE SETTINGS</h2></div><button className="settings-close" onClick={onClose}>×</button></header>
    <div className="settings-scroll">
      <div className="section-head">OPENAI-COMPATIBLE LLM</div>
      <label>Base URL<input value={draft.llm_base_url} onChange={e => setDraft({ ...draft, llm_base_url: e.target.value })} placeholder="http://127.0.0.1:11434/v1"/></label>
      <label>Model<input value={draft.llm_model} onChange={e => setDraft({ ...draft, llm_model: e.target.value })} placeholder="qwen3:8b"/></label>
      <label>API key<input type="password" value={draft.llm_api_key} onChange={e => setDraft({ ...draft, llm_api_key: e.target.value })} placeholder="Optional for local endpoints"/></label>
      <label>Temperature<input type="number" min="0" max="2" step="0.1" value={draft.temperature} onChange={e => setDraft({ ...draft, temperature: Number(e.target.value) || 0 })}/></label>
      <div className="section-head setting-gap">COMFYUI</div>
      <label>API URL<input value={draft.comfyui_url} onChange={e => setDraft({ ...draft, comfyui_url: e.target.value })} placeholder="http://127.0.0.1:8188"/></label>
      <label>API workflow template<small className="settings-hint">Use placeholders POSITIVE_PROMPT, NEGATIVE_PROMPT, SEED, STORY_ID and SCENE_ID inside the API-format workflow JSON.</small><textarea className="workflow-input" value={draft.comfyui_workflow_json} onChange={e => setDraft({ ...draft, comfyui_workflow_json: e.target.value })} placeholder='Paste a ComfyUI API workflow JSON template here...'/></label>
    </div>
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
      setStory(await api.getStory(id));
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => { void loadState().catch(e => setError(String(e))); }, []);
  useEffect(() => {
    let disposed = false;
    const apply = (next: RegistryStatusDto) => {
      if (!disposed) setRegistryStatus(next);
    };

    setRegistryStatus(current => ({ ...current, status: 'starting', detail: 'Starting Raphael Model Registry…' }));
    void api.ensureRegistry()
      .then(apply)
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

  useEffect(() => { if (!story && state?.stories[0]) void selectStory(state.stories[0].id); }, [state?.stories]);

  const generateStory = async () => { if (!prompt.trim() || busy) return; setBusy(true); setError(null); try { const next = await api.createStory(prompt.trim()); setStory(next); setPrompt(''); await loadState(); } catch (e) { setError(String(e)); } finally { setBusy(false); } };
  const generateNext = async () => { if (!story || busy) return; setBusy(true); setError(null); try { const next = await api.generateNextChapter(story.id, directive.trim()); setStory(next); setDirective(''); await loadState(); } catch (e) { setError(String(e)); } finally { setBusy(false); } };
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
      setError(String(e));
    } finally {
      setExtracting(false);
    }
  };
  const buildPrompt = async (scene: Scene) => { if (!story || buildingPrompt) return; const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id)); if (!chapter) return; setBuildingPrompt(scene.id); setError(null); try { setStory(await api.buildScenePrompt(story.id, chapter.number, scene.id)); await loadState(); } catch (e) { setError(String(e)); } finally { setBuildingPrompt(null); } };
  const viewPrompt = (scene: Scene) => setPromptPreview(scene);
  const queueImage = async (scene: Scene) => { if (!story || queueing) return; const chapter = story.chapters.find(c => c.scenes.some(s => s.id === scene.id)); if (!chapter) return; setQueueing(scene.id); setError(null); try { setStory(await api.queueSceneImage(story.id, chapter.number, scene.id)); await loadState(); } catch (e) { setError(String(e)); } finally { setQueueing(null); } };

  const currentChapter = story?.chapters[story.chapters.length - 1] || null;
  const totalScenes = useMemo(() => story?.chapters.reduce((n, c) => n + c.scenes.length, 0) || 0, [story]);

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
              void loadState().catch(e => setError(String(e)));
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
    <header className="topbar">
      <div className="brand"><PulseMark/><div><div className="brand-title">RAPHAEL</div><div className="brand-subtitle">STORY GENERATOR</div></div></div>
      <div className="top-status"><span className={state.llm_configured ? 'status-ok' : 'status-warn'}>LLM {state.llm_configured ? 'READY' : 'NOT CONFIGURED'}</span><span className={`registry-health ${registryStatus.status}`} title={registryStatus.detail || registryStatus.url}><i />REGISTRY {registryStatus.status === 'on' ? 'ON' : registryStatus.status === 'starting' ? 'STARTING' : 'OFF'}</span><span>SCENES {totalScenes}</span><button className="icon-btn" onClick={() => setSettingsOpen(true)} title="Engine settings">⚙</button></div>
    </header>

    <div className="workspace">
      <aside className="sidebar">
        <div className="sidebar-head"><div><div className="eyebrow">LIBRARY</div><h2>STORIES</h2></div><button className="square-btn" disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing)} onClick={() => { setStory(null); setPrompt(''); setDirective(''); setPromptPreview(null); }}>+</button></div>
        <div className="story-list">{state.stories.length === 0 ? <div className="empty-sidebar">No stories yet.<br/>Create the first one from the story prompt.</div> : state.stories.map(item => <StoryCard key={item.id} story={item} selected={story?.id === item.id} disabled={busy || extracting || Boolean(buildingPrompt) || Boolean(queueing)} onClick={() => void selectStory(item.id)}/>)}</div>
        <div className="sidebar-foot">LOCAL-FIRST · CANON SAVED TO APP DATA</div>
      </aside>

      <main className="main-panel">
        {error ? <div className="error-box">{error}</div> : null}
        {!story ? <NewStoryPanel busy={busy} prompt={prompt} setPrompt={setPrompt} onGenerate={() => void generateStory()}/> : <>
          <section className="story-header hud-panel">
            <div><div className="eyebrow">STORY BIBLE · {story.id.slice(0, 8).toUpperCase()}</div><h1>{story.title}</h1><p>{story.bible.premise}</p></div>
            <div className="story-header-tags"><TagRow values={story.metadata.tone} tone="tone"/></div>
          </section>
          <IntroductionView story={story}/>
          {currentChapter ? <ChapterView story={story} chapter={currentChapter} onExtractScenes={() => void extractScenes(currentChapter)} extracting={extracting} onBuildPrompt={scene => void buildPrompt(scene)} onViewPrompt={viewPrompt} onQueueImage={queueImage} buildingPrompt={buildingPrompt} queueing={queueing}/> : null}
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
        <RegistryPanel status={registryStatus} catalog={registryCatalog} error={registryCatalogError}/>
        {story ? <MetadataPanel story={story}/> : <section className="hud-panel inspector-panel placeholder-inspector"><div className="section-head">PIPELINE</div><div className="pipeline-step active"><b>01</b><span>STORY BIBLE</span></div><div className="pipeline-step"><b>02</b><span>CHAPTERS</span></div><div className="pipeline-step"><b>03</b><span>SCENE EXTRACTION</span></div><div className="pipeline-step"><b>04</b><span>IMAGE BUILDER</span></div><div className="pipeline-note">The story engine is designed so scene extraction and ComfyUI image generation can run without changing the story canon.</div></section>}
      </aside>
    </div>

    {promptPreview ? <div className="overlay" onMouseDown={() => setPromptPreview(null)}><section className="prompt-viewer hud-panel" onMouseDown={e => e.stopPropagation()}><header className="settings-header"><div><div className="eyebrow">IMAGE BUILDER</div><h2>SCENE {String(promptPreview.order).padStart(2, '0')} PROMPTS</h2></div><button className="settings-close" onClick={() => setPromptPreview(null)}>×</button></header><div className="prompt-viewer-body"><div><div className="section-head">POSITIVE PROMPT</div><pre className="prompt-box">{promptPreview.positive_prompt}</pre></div><div><div className="section-head">NEGATIVE PROMPT</div><pre className="prompt-box">{promptPreview.negative_prompt}</pre></div></div></section></div> : null}
    {settingsOpen ? <SettingsOverlay
      settings={state.settings}
      onSave={async value => {
        const next = await api.saveSettings(value);
        setState(prev => prev ? { ...prev, settings: next, llm_configured: Boolean(next.llm_base_url && next.llm_model) } : prev);
      }}
      onError={message => setError(message)}
      onClose={() => setSettingsOpen(false)}
    /> : null}
  </div>;
}
