<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { initRouter, router } from "./lib/router.svelte";
  import Detail from "./pages/Detail.svelte";
  import Home from "./pages/Home.svelte";

  $effect(() => initRouter());
</script>

<svelte:head>
  <title>rust-svelte-template</title>
</svelte:head>

<Header />

<main>
  {#if router.index === 1}
    <Detail id={router.params.id} />
  {:else}
    <Home />
  {/if}
</main>
