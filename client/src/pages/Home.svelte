<script lang="ts">
  import Icon from "../lib/Icon.svelte";
  import { fetchItems, type Item } from "../lib/api";

  type ListState = "loading" | "empty" | "error" | "success";

  let items = $state<Item[]>([]);
  let query = $state("");
  let matches = $derived(
    items.filter((item) =>
      item.name.toLowerCase().includes(query.toLowerCase()),
    ),
  );
  let listState = $state<ListState>("loading");

  let controller: AbortController | undefined;
  let loadedOnce = false;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    if (!loadedOnce) {
      listState = "loading";
    }
    try {
      items = await fetchItems(controller.signal);
      listState = items.length === 0 ? "empty" : "success";
      loadedOnce = true;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      listState = "error";
    }
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") {
      void load();
    }
  }

  $effect(() => {
    void load();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<section class="content" data-state={listState}>
  <label for="item-search">名前で検索</label>
  <div class="search-field">
    <span class="search-icon"><Icon name="search" /></span>
    <input id="item-search" type="search" bind:value={query} />
  </div>
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">項目がありません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}>
        再試行
      </button>
    </div>
  {:else if matches.length === 0}
    <p class="state" role="status">一致する項目がありません</p>
  {:else}
    <ul class="cards">
      {#each matches as item (item.id)}
        <li>
          <a class="card" href={`/items/${item.id}`}>
            <span class="name">{item.name}</span>
            <span class="updated">{item.updated_at}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style lang="sass">
  label
    display: block
    margin-bottom: var(--sp-2)
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4

  .search-field
    position: relative
    margin-bottom: var(--sp-3)

  .search-icon
    position: absolute
    left: var(--sp-2)
    top: 50%
    transform: translateY(-50%)
    color: var(--c-muted)
    pointer-events: none

  input
    width: 100%
    min-width: 0
    padding: var(--sp-2)
    padding-left: calc(var(--sp-2) * 2 + 1.2em)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-size: var(--fs-lg)
    line-height: 1.6

    &:focus
      border-color: var(--c-accent)

  .cards
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: 0
    padding: 0
    list-style: none

  .card
    display: flex
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-2)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    color: var(--c-on-surface)
    text-decoration: none

    &:hover
      background: var(--c-hover-1)

  .name
    min-width: 0
    overflow-wrap: anywhere
    font-size: var(--fs-md)
    font-weight: 500

  .updated
    flex-shrink: 0
    font-size: var(--fs-xs)
    color: var(--c-muted)
</style>
