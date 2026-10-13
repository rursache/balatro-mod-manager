<script lang="ts">
  import { ChevronDown } from "lucide-svelte";
  import { fade, slide } from "svelte/transition";

  interface Option {
    value: string;
    label: string;
  }

  let {
    options,
    value = $bindable(""),
    disabled = false,
    placeholder = "Select an option",
  }: {
    options: Option[];
    value: string;
    disabled?: boolean;
    placeholder?: string;
  } = $props();

  let isOpen = $state(false);
  let dropdownRef = $state<HTMLDivElement | null>(null);

  const selectedLabel = $derived(
    options.find((o) => o.value === value)?.label ?? placeholder,
  );

  function toggle() {
    if (!disabled) {
      isOpen = !isOpen;
    }
  }

  function select(optionValue: string) {
    value = optionValue;
    isOpen = false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (disabled) return;

    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      toggle();
    } else if (e.key === "Escape" && isOpen) {
      // Claim the key so the view or popup behind stays open
      e.preventDefault();
      isOpen = false;
    } else if (e.key === "ArrowDown" && isOpen) {
      e.preventDefault();
      const currentIndex = options.findIndex((o) => o.value === value);
      const nextIndex = Math.min(currentIndex + 1, options.length - 1);
      value = options[nextIndex].value;
    } else if (e.key === "ArrowUp" && isOpen) {
      e.preventDefault();
      const currentIndex = options.findIndex((o) => o.value === value);
      const prevIndex = Math.max(currentIndex - 1, 0);
      value = options[prevIndex].value;
    }
  }

  function handleClickOutside(e: MouseEvent) {
    if (dropdownRef && !dropdownRef.contains(e.target as Node)) {
      isOpen = false;
    }
  }
</script>

<svelte:window onclick={handleClickOutside} />

<div
  class="dropdown"
  class:disabled
  class:open={isOpen}
  bind:this={dropdownRef}
>
  <button
    type="button"
    class="dropdown-trigger"
    onclick={toggle}
    onkeydown={handleKeydown}
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={isOpen}
  >
    <span class="selected-text">{selectedLabel}</span>
    <ChevronDown size={16} class="chevron" />
  </button>

  {#if isOpen}
    <div
      class="dropdown-menu"
      role="listbox"
      transition:slide={{ duration: 120 }}
    >
      {#each options as option (option.value)}
        <button
          type="button"
          class="dropdown-item"
          class:selected={option.value === value}
          onclick={() => select(option.value)}
          role="option"
          aria-selected={option.value === value}
        >
          {option.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dropdown {
    position: relative;
    width: 100%;
  }

  .dropdown-trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 0.75rem;
    background: var(--ui-danger-overlay);
    color: var(--ui-text);
    border: 1px solid var(--ui-danger-overlay-border);
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .dropdown-trigger:hover:not(:disabled) {
    background-color: var(--ui-danger-overlay-stronger);
    border-color: var(--ui-danger-overlay-border-strong);
    transform: translateY(-2px);
  }

  .dropdown.open .dropdown-trigger {
    border-color: var(--ui-danger-overlay-border-strong);
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
  }

  .dropdown-trigger:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .selected-text {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dropdown-trigger :global(.chevron) {
    flex-shrink: 0;
    transition: transform 0.2s ease;
  }

  .dropdown.open .dropdown-trigger :global(.chevron) {
    transform: rotate(180deg);
  }

  .dropdown-menu {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    background: var(--ui-danger-overlay-stronger);
    border: 1px solid var(--ui-danger-overlay-border-strong);
    border-top: none;
    border-bottom-left-radius: 6px;
    border-bottom-right-radius: 6px;
    max-height: 200px;
    overflow-y: auto;
    z-index: 100;
  }

  .dropdown-menu::-webkit-scrollbar {
    width: 6px;
  }

  .dropdown-menu::-webkit-scrollbar-track {
    background: transparent;
  }

  .dropdown-menu::-webkit-scrollbar-thumb {
    background: var(--ui-scroll-thumb);
    border-radius: 3px;
  }

  .dropdown-item {
    display: block;
    width: 100%;
    padding: 0.6rem 0.75rem;
    background: transparent;
    color: var(--ui-text);
    border: none;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    text-align: left;
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  .dropdown-item:hover {
    background: var(--ui-danger-overlay-border);
  }

  .dropdown-item.selected {
    background: var(--ui-danger-overlay-border);
    color: var(--ui-success);
  }

  .disabled {
    pointer-events: none;
  }
</style>
