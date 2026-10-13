import { writable, type Readable } from "svelte/store";
import {
  FOCUSABLE_SELECTOR,
  focusControllerElement,
  isControllerFocusActive,
} from "./controllerNavigation";

export interface ModalOptions {
  // Called when Escape (or controller B) reaches this modal while it is on top
  onEscape?: () => void;
  // Selector for the element that receives focus on open, defaults to [data-autofocus]
  initialFocus?: string;
}

interface ModalEntry {
  element: HTMLElement;
  options: ModalOptions;
  previousFocus: HTMLElement | null;
}

const stack: ModalEntry[] = [];
const depth = writable(0);
let listening = false;

export const modalOpen: Readable<boolean> = {
  subscribe: (run) => depth.subscribe((value) => run(value > 0)),
};

// Svelte marks an outroing element inert right away but fires outrostart a frame later
function isClosing(entry: ModalEntry): boolean {
  return entry.element.inert || entry.element.closest("[inert]") !== null;
}

function liveTop(): ModalEntry | null {
  for (let i = stack.length - 1; i >= 0; i--) {
    if (!isClosing(stack[i])) return stack[i];
  }
  return null;
}

export function isModalOpen(): boolean {
  return liveTop() !== null;
}

export function getTopModal(): HTMLElement | null {
  return liveTop()?.element ?? null;
}

function isTabbable(element: HTMLElement): boolean {
  if (element.closest("[hidden], [inert], [aria-hidden='true']")) return false;
  if (element.getAttribute("aria-disabled") === "true") return false;
  if ("disabled" in element && Boolean(element.disabled)) return false;
  if (element.tabIndex < 0) return false;
  const style = window.getComputedStyle(element);
  return style.display !== "none" && style.visibility !== "hidden";
}

export function getTabbableElements(root: ParentNode): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
  ).filter(isTabbable);
}

export function getTrappedTabTarget(
  elements: HTMLElement[],
  active: Element | null,
  backwards: boolean,
): HTMLElement | null {
  if (elements.length === 0) return null;
  const index = active ? elements.indexOf(active as HTMLElement) : -1;
  if (index === -1)
    return backwards ? elements[elements.length - 1] : elements[0];
  const next = backwards ? index - 1 : index + 1;
  return elements[(next + elements.length) % elements.length];
}

export function getInitialFocusTarget(
  root: HTMLElement,
  selector = "[data-autofocus]",
): HTMLElement | null {
  const preferred = root.querySelector<HTMLElement>(selector);
  if (preferred && isTabbable(preferred)) return preferred;
  return getTabbableElements(root)[0] ?? null;
}

function applyFocus(element: HTMLElement, controller: boolean) {
  if (controller && focusControllerElement(element)) return;
  element.focus({ preventScroll: true });
}

function handleKeydown(event: KeyboardEvent) {
  const top = liveTop();
  if (!top || event.defaultPrevented) return;

  if (event.key === "Escape" || event.key === "Esc") {
    event.preventDefault();
    event.stopImmediatePropagation();
    top.options.onEscape?.();
    return;
  }

  if (event.key === "Tab") {
    event.preventDefault();
    const target = getTrappedTabTarget(
      getTabbableElements(top.element),
      document.activeElement,
      event.shiftKey,
    );
    (target ?? top.element).focus({ preventScroll: true });
  }
}

function ensureListener() {
  if (listening || typeof window === "undefined") return;
  window.addEventListener("keydown", handleKeydown);
  listening = true;
}

function pushModal(element: HTMLElement, options: ModalOptions): ModalEntry {
  ensureListener();
  const active = document.activeElement;
  const entry: ModalEntry = {
    element,
    options,
    previousFocus:
      active instanceof HTMLElement && active !== document.body ? active : null,
  };
  stack.push(entry);
  depth.set(stack.length);

  const controller = isControllerFocusActive();
  const target = getInitialFocusTarget(element, options.initialFocus);
  if (target) applyFocus(target, controller);
  return entry;
}

function removeModal(entry: ModalEntry) {
  const index = stack.indexOf(entry);
  if (index === -1) return;
  const wasTop = index === stack.length - 1;
  stack.splice(index, 1);
  depth.set(stack.length);

  if (!wasTop) {
    // Hand the focus origin up so closing the modal above still lands somewhere valid
    const above = stack[index];
    if (above?.previousFocus && entry.element.contains(above.previousFocus)) {
      above.previousFocus = entry.previousFocus;
    }
    return;
  }

  const active = document.activeElement;
  const focusInside =
    !active || active === document.body || entry.element.contains(active);
  const previous = entry.previousFocus;
  if (focusInside && previous?.isConnected) {
    applyFocus(previous, isControllerFocusActive());
  }
}

// Svelte action: put it on the element that owns the popup's outro transition
export function modal(node: HTMLElement, options: ModalOptions = {}) {
  let current = options;
  let entry: ModalEntry | null = pushModal(node, current);

  const release = () => {
    if (entry) removeModal(entry);
    entry = null;
  };
  const reopen = () => {
    if (!entry) entry = pushModal(node, current);
  };

  // A closing popup stays in the DOM while it fades out, it must not keep the trap
  node.addEventListener("outrostart", release);
  node.addEventListener("introstart", reopen);

  return {
    update(next: ModalOptions = {}) {
      current = next;
      if (entry) entry.options = next;
    },
    destroy() {
      node.removeEventListener("outrostart", release);
      node.removeEventListener("introstart", reopen);
      release();
    },
  };
}

export function resetModalStack() {
  stack.length = 0;
  depth.set(0);
}
