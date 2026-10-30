<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { initRouter, router } from "./lib/router.svelte";
  import Home from "./pages/Home.svelte";
  import View from "./pages/View.svelte";

  $effect(() => initRouter());
</script>

<svelte:head>
  <title>blender-live</title>
</svelte:head>

<Header />

<main>
  {#if router.index === 1}
    <View path={router.params.path} />
  {:else}
    <Home />
  {/if}
</main>
