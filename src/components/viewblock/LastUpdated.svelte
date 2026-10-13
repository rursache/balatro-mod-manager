<script lang="ts">
  import { Clock } from "lucide-svelte";
  import { formatLastUpdated } from "../../utils/lastUpdated";

  interface Props {
    value?: number | null;
    class?: string;
    iconSize?: number;
  }

  let { value, class: className = "", iconSize = 16 }: Props = $props();

  let label = $derived(formatLastUpdated(value));
</script>

{#if label}
  <span class={className} title="Last updated" data-testid="last-updated">
    <Clock size={iconSize} />
    {label}
  </span>
{/if}
