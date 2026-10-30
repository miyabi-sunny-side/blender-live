<script lang="ts">
  import IconCatalog from "../lib/IconCatalog.svelte";
  import { fetchItem, type Item } from "../lib/api";

  let { id }: { id: string } = $props();

  let item = $state<Item | undefined>();
  let detailState = $state<"loading" | "error" | "success">("loading");

  let controller: AbortController | undefined;

  async function load(currentId: string) {
    controller?.abort();
    controller = new AbortController();
    try {
      item = await fetchItem(currentId, controller.signal);
      detailState = "success";
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      detailState = "error";
    }
  }

  $effect(() => {
    void load(id);
    return () => controller?.abort();
  });
</script>

<div class="sub-header">
  <h1 class="sub-title">{item ? item.name : "詳細"}</h1>
</div>

<div class="content">
  {#if detailState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if detailState === "error"}
    <p class="state error">読み込みに失敗しました</p>
  {:else if item}
    <p class="summary">{item.summary}</p>
    <p class="meta">
      <span class="badge">{item.status}</span>
      <span class="updated">{item.updated_at}</span>
    </p>
    <p class="body-text">{item.body}</p>
    {#if item.id === "icons"}
      <IconCatalog />
    {/if}
  {/if}
</div>

<style lang="sass">
  .sub-header
    position: sticky
    top: var(--header-h)
    z-index: 9
    display: flex
    align-items: center
    gap: var(--sp-2)
    height: var(--subheader-h)
    padding: 0 var(--sp-3)
    background: var(--c-wash-raised)
    border-bottom: 1px solid var(--c-border)

  .sub-title
    margin: 0
    overflow: hidden
    text-overflow: ellipsis
    white-space: nowrap
    font-size: var(--fs-md)
    font-weight: 500
    line-height: 1.2

  .summary
    margin: 0 0 var(--sp-3)

  .meta
    display: flex
    align-items: center
    gap: var(--sp-2)
    margin: 0 0 var(--sp-4)

  .badge
    display: inline-block
    padding: var(--sp-1) var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-full)
    font-size: var(--fs-xs)
    line-height: 1.4
    color: var(--c-muted)

  .updated
    font-size: var(--fs-xs)
    color: var(--c-muted)

  .body-text
    margin: 0
    white-space: pre-line
</style>
