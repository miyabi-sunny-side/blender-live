<script lang="ts">
  import Icon from "../lib/Icon.svelte";
  import {
    cancelJob,
    fetchFiles,
    type Dir,
    type Entry,
    type Job,
    type Status,
  } from "../lib/api";
  import {
    basename,
    fileUrl,
    formatSize,
    formatTime,
    phaseLabel,
    processLabel,
    progressPercent,
    totalFrames,
    viewHref,
  } from "../lib/format";

  type ListState = "loading" | "empty" | "error" | "success";
  const TABS: Dir[] = ["scenes", "scripts", "assets"];

  let status = $state<Status | null>(null);
  let jobs = $state<Job[]>([]);
  let renders = $state<Entry[]>([]);
  let rendersState = $state<ListState>("loading");
  let tab = $state<Dir>("scenes");
  let tabFiles = $state<Entry[]>([]);
  let tabState = $state<ListState>("loading");
  let now = $state(Date.now());

  let visibleJobs = $derived(jobs.slice(0, 5));

  async function loadRenders() {
    try {
      renders = await fetchFiles("renders");
      rendersState = renders.length === 0 ? "empty" : "success";
    } catch {
      rendersState = "error";
    }
  }

  async function loadTab(dir: Dir) {
    try {
      const entries = await fetchFiles(dir);
      if (dir === tab) {
        tabFiles = entries;
        tabState = entries.length === 0 ? "empty" : "success";
      }
    } catch {
      tabState = "error";
    }
  }

  function reload() {
    now = Date.now();
    void loadRenders();
    void loadTab(tab);
  }

  function selectTab(dir: Dir) {
    tab = dir;
    tabState = "loading";
    void loadTab(dir);
  }

  $effect(() => {
    reload();
    const source = new EventSource("/api/events");
    source.onopen = reload;
    source.onmessage = (message: MessageEvent<string>) => {
      const event = JSON.parse(message.data) as
        | { type: "status"; data: Status }
        | { type: "jobs"; data: Job[] }
        | { type: "files" };
      if (event.type === "status") {
        status = event.data;
      } else if (event.type === "jobs") {
        jobs = event.data;
      } else {
        reload();
      }
    };
    return () => source.close();
  });
</script>

<div class="content content-wide">
  <section class="process" aria-label="Blender の状態">
    {#if status}
      <span class="badge proc" data-state={status.state}>
        {processLabel(status.state)}
      </span>
      <span class="file">{status.file ?? "新しい scene"}</span>
      {#if status.unsaved}
        <span class="badge">未保存</span>
      {/if}
      {#if status.busy_with}
        <span class="muted">{status.busy_with}</span>
      {/if}
      <span class="muted gpu">
        Blender {status.version} · Cycles {status.devices.join(", ") || "CPU"}
        {#if status.device_type}({status.device_type}){/if}
      </span>
    {:else}
      <span class="muted"
        ><span class="spinner" aria-hidden="true"></span>接続中…</span
      >
    {/if}
  </section>

  {#if status?.lost}
    <div class="error-banner" role="alert">
      <p>
        {formatTime(status.lost.at_ms, now)} に Blender が落ちたため起こし直しました。
        {status.lost.unsaved
          ? "未保存の scene は失われました。"
          : "失われた変更はありません。"}
        {#if status.lost.file}{status.lost.file} を開き直しました。{/if}
      </p>
      {#if status.last_error}
        <details>
          <summary>詳細</summary>
          <pre>{status.last_error}</pre>
        </details>
      {/if}
    </div>
  {/if}

  {#if visibleJobs.length > 0}
    <section aria-labelledby="jobs-heading">
      <h2 id="jobs-heading">ジョブ</h2>
      <ul class="jobs">
        {#each visibleJobs as job (job.id)}
          {@const active =
            job.phase === "rendering" || job.phase === "encoding"}
          <li class="job" data-phase={job.phase}>
            <div class="job-line">
              {#if job.phase === "done"}
                <a class="name" href={viewHref(`renders/${job.name}.mp4`)}
                  >{job.name}</a
                >
              {:else}
                <span class="name">{job.name}</span>
              {/if}
              <span class="badge">{phaseLabel(job)}</span>
              <span class="muted frames"
                >{job.done_frames} / {totalFrames(job)}</span
              >
              {#if active && !job.cancel_requested}
                <button
                  class="icon-btn"
                  type="button"
                  aria-label={`${job.name} を取り消す`}
                  onclick={() => void cancelJob(job.id)}
                >
                  <Icon name="x" />
                </button>
              {/if}
            </div>
            <div
              class="bar"
              role="progressbar"
              aria-label={`${job.name} の進み具合`}
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={progressPercent(job)}
            >
              <span style:width={`${progressPercent(job)}%`}></span>
            </div>
            {#if job.error}
              <p class="error">{job.error}</p>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section aria-labelledby="renders-heading" data-state={rendersState}>
    <h2 id="renders-heading">書き出し</h2>
    {#if rendersState === "loading"}
      <p class="state">
        <span class="spinner" aria-hidden="true"></span>読み込み中…
      </p>
    {:else if rendersState === "empty"}
      <p class="state">まだ書き出しがありません</p>
    {:else if rendersState === "error"}
      <div class="state-wrap">
        <p class="state error">読み込みに失敗しました</p>
        <button class="btn" type="button" onclick={reload}>再試行</button>
      </div>
    {:else}
      <ul class="gallery">
        {#each renders as entry (entry.path)}
          <li class="render" data-kind={entry.kind}>
            {#if entry.kind === "video"}
              <!-- svelte-ignore a11y_media_has_caption -->
              <video
                src={fileUrl(entry.path)}
                controls
                preload="metadata"
                playsinline
              ></video>
            {:else if entry.kind === "image" || entry.kind === "sequence"}
              <a
                class="media"
                href={viewHref(entry.cover ?? entry.path)}
                tabindex="-1"
              >
                <img
                  src={fileUrl(entry.cover ?? entry.path)}
                  alt=""
                  loading="lazy"
                />
              </a>
            {/if}
            <div class="caption">
              <a class="name" href={viewHref(entry.cover ?? entry.path)}
                >{basename(entry.path)}</a
              >
              <span class="muted meta">
                {formatTime(entry.modified_ms, now)}
                {#if entry.frames}· {entry.frames} frames{:else}· {formatSize(
                    entry.size,
                  )}{/if}
              </span>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section aria-labelledby="files-heading">
    <h2 id="files-heading">ファイル</h2>
    <div class="tabs" role="tablist" aria-labelledby="files-heading">
      {#each TABS as dir (dir)}
        <button
          type="button"
          role="tab"
          aria-selected={tab === dir}
          onclick={() => selectTab(dir)}>{dir}</button
        >
      {/each}
    </div>
    <div role="tabpanel" data-state={tabState}>
      {#if tabState === "loading"}
        <p class="state">
          <span class="spinner" aria-hidden="true"></span>読み込み中…
        </p>
      {:else if tabState === "empty"}
        <p class="state">ファイルがありません</p>
      {:else if tabState === "error"}
        <p class="state error">読み込みに失敗しました</p>
      {:else}
        <ul class="files">
          {#each tabFiles as entry (entry.path)}
            <li>
              <span class="path">{entry.path.slice(tab.length + 1)}</span>
              <span class="muted meta"
                >{formatSize(entry.size)} · {formatTime(
                  entry.modified_ms,
                  now,
                )}</span
              >
              <a
                class="icon-btn"
                href={fileUrl(entry.path, true)}
                download
                aria-label={`${basename(entry.path)} をダウンロード`}
              >
                <Icon name="download" />
              </a>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </section>
</div>

<style lang="sass">
  h2
    margin: var(--sp-5) 0 var(--sp-2)
    font-size: var(--fs-xl)
    font-weight: 600
    line-height: 1.3

  .muted
    color: var(--c-muted)
    font-size: var(--fs-sm)

  .badge
    padding: 2px var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-full)
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4
    white-space: nowrap

  .proc[data-state="busy"], .proc[data-state="starting"], .proc[data-state="restarting"]
    color: var(--c-accent)
    border-color: var(--c-accent)

  .proc[data-state="failed"]
    color: var(--c-danger)
    border-color: var(--c-danger)

  .process
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  .file
    min-width: 0
    overflow-wrap: anywhere
    font-weight: 500

  .gpu
    flex-basis: 100%

  .error-banner
    margin-top: var(--sp-3)
    padding: var(--sp-2)
    border-radius: var(--radius-sm)
    background: var(--c-danger-subtle)
    color: var(--c-danger)
    font-size: var(--fs-sm)
    line-height: 1.5

    p
      margin: 0

    pre
      margin: var(--sp-2) 0 0
      white-space: pre-wrap
      overflow-wrap: anywhere

  ul
    margin: 0
    padding: 0
    list-style: none

  .jobs
    display: grid
    gap: var(--sp-2)

  .job
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  .job-line
    display: flex
    align-items: center
    gap: var(--sp-2)
    min-height: 36px

    .name
      min-width: 0
      overflow-wrap: anywhere
      font-weight: 500

    .frames
      margin-left: auto
      font-variant-numeric: tabular-nums

  .bar
    height: 6px
    margin-top: var(--sp-2)
    border-radius: var(--radius-full)
    background: var(--c-hover-1)
    overflow: hidden

    span
      display: block
      height: 100%
      background: var(--c-accent)
      transition: width 150ms

  .error
    margin: var(--sp-2) 0 0
    color: var(--c-danger)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  .gallery
    display: grid
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr))
    gap: var(--sp-3)

  .render
    display: flex
    flex-direction: column
    min-width: 0
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    overflow: hidden

    img, video
      display: block
      width: 100%
      aspect-ratio: 16 / 9
      object-fit: contain
      background: #000

  .caption
    display: flex
    flex-direction: column
    padding: var(--sp-2) 10px

    .name
      overflow: hidden
      text-overflow: ellipsis
      white-space: nowrap

  .tabs
    display: flex
    gap: var(--sp-1)
    margin-bottom: var(--sp-2)
    border-bottom: 1px solid var(--c-border)

    button
      padding: var(--sp-2) var(--sp-3)
      border: none
      border-bottom: 2px solid transparent
      background: transparent
      color: var(--c-muted)
      font-size: var(--fs-md)
      cursor: pointer

      &[aria-selected="true"]
        border-bottom-color: var(--c-accent)
        color: var(--c-on-surface)
        font-weight: 500

      &:hover
        background: var(--c-hover-1)

  .files li
    display: flex
    align-items: center
    gap: var(--sp-2)
    padding: var(--sp-1) 0
    border-bottom: 1px solid var(--c-border)

    .path
      flex: 1
      min-width: 0
      overflow-wrap: anywhere

    .meta
      white-space: nowrap

  @media (max-width: 480px)
    .gallery
      grid-template-columns: repeat(2, minmax(0, 1fr))
      gap: var(--sp-2)

    .files .meta
      display: none

</style>
