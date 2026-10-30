<script lang="ts">
  import Icon from "../lib/Icon.svelte";
  import { basename, fileUrl, mediaKind } from "../lib/format";

  let { path }: { path: string } = $props();
  let name = $derived(basename(path));
  let kind = $derived(mediaKind(path));
</script>

<h1 class="sub-header">{name}</h1>
<div class="content content-wide">
  {#if kind === "image"}
    <img src={fileUrl(path)} alt={name} />
  {:else if kind === "video"}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video src={fileUrl(path)} controls playsinline></video>
  {/if}
  <div class="actions">
    <span class="muted">{path}</span>
    <a class="btn" href={fileUrl(path, true)} download>
      <Icon name="download" />ダウンロード
    </a>
  </div>
</div>

<style lang="sass">
  .sub-header
    position: sticky
    top: var(--header-h)
    z-index: 1
    height: var(--subheader-h)
    margin: 0
    padding: 0 var(--sp-3)
    border-bottom: 1px solid var(--c-border)
    background: var(--c-wash-raised)
    font-size: var(--fs-lg)
    font-weight: 600
    line-height: var(--subheader-h)
    overflow: hidden
    text-overflow: ellipsis
    white-space: nowrap

  img, video
    display: block
    width: 100%
    max-height: 75dvh
    object-fit: contain
    border-radius: var(--radius-md)
    background: #000

  .actions
    display: flex
    flex-wrap: wrap
    align-items: center
    justify-content: space-between
    gap: var(--sp-2)
    margin-top: var(--sp-3)

  .muted
    min-width: 0
    color: var(--c-muted)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  .btn
    display: inline-flex
    align-items: center
    gap: var(--sp-2)
    text-decoration: none
</style>
