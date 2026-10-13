/**
 * Centralized timing constants for the application.
 * All timeout and interval values should be defined here for consistency.
 */

// =============================================================================
// Persistence & Storage
// =============================================================================

/** Debounce delay for persisting mod store to localStorage (ms) */
export const MOD_STORE_PERSIST_DEBOUNCE_MS = 2000;

/** Timeout for mod store persistence operation (ms) */
export const MOD_STORE_PERSIST_TIMEOUT_MS = 5000;

/** Debounce delay for persisting descriptions to localStorage (ms) */
export const DESCRIPTIONS_PERSIST_DEBOUNCE_MS = 1500;

/** Timeout for descriptions persistence operation (ms) */
export const DESCRIPTIONS_PERSIST_TIMEOUT_MS = 2000;

// =============================================================================
// Catalog & Data Loading
// =============================================================================

/** Base delay for catalog fetch retry with exponential backoff (ms) */
export const CATALOG_RETRY_BASE_DELAY_MS = 5000;

/** Maximum delay for catalog fetch retry (ms) */
export const CATALOG_RETRY_MAX_DELAY_MS = 60000;

/** Jitter range for catalog retry to avoid thundering herd (ms) */
export const CATALOG_RETRY_JITTER_MS = 1000;

// =============================================================================
// UI Timers
// =============================================================================

/** Delay before showing loading indicator for local mods (ms) */
export const LOCAL_MODS_LOADING_DELAY_MS = 300;

/** Idle timeout before updating pagination (ms) */
export const PAGINATION_IDLE_TIMEOUT_MS = 150;

/** Idle timeout for scroll position tracking (ms) */
export const SCROLL_IDLE_TIMEOUT_MS = 100;

/** Delay for hydrating visible mods after scroll (ms) */
export const VISIBLE_HYDRATE_DELAY_MS = 50;

/** Debounce delay for title scaling on resize (ms) */
export const RESIZE_DEBOUNCE_MS = 50;

/** Animation interval for loading dots (ms) */
export const LOADING_DOTS_INTERVAL_MS = 500;

// =============================================================================
// Image Loading (LazyImage)
// =============================================================================

/** Delay before showing spinner during image load (ms) */
export const IMAGE_SPINNER_DELAY_MS = 150;

/** Delay before starting image load after intersection (ms) */
export const IMAGE_LOAD_DELAY_MS = 50;

/** Delay for rechecking cache after queue submission (ms) */
export const IMAGE_CACHE_RECHECK_DELAY_MS = 500;

// =============================================================================
// Launch Detection
// =============================================================================

/** Interval for checking if game process started (ms) */
export const LAUNCH_CHECK_INTERVAL_MS = 500;

/** Timeout for waiting for game to launch (ms) */
export const LAUNCH_TIMEOUT_MS = 10000;

// =============================================================================
// Navigation & Transitions
// =============================================================================

/** Timeout for page navigation operations (ms) */
export const NAV_TIMEOUT_MS = 4000;

/** Delay before navigating after picker selection (ms) */
export const PICKER_NAV_DELAY_MS = 1000;

/** Duration for confetti animation (ms) */
export const CONFETTI_DURATION_MS = 2000;

// =============================================================================
// Tauri IPC
// =============================================================================

/** Default timeout for Tauri invoke calls (ms) */
export const TAURI_INVOKE_TIMEOUT_MS = 5000;

// =============================================================================
// Popup Management
// =============================================================================

/** Delay for popup show transition (ms) */
export const POPUP_SHOW_DELAY_MS = 10;

/** Delay for popup hide transition (ms) */
export const POPUP_HIDE_DELAY_MS = 200;

// =============================================================================
// Version Cache
// =============================================================================

/** Duration to cache version info (ms) */
export const VERSION_CACHE_DURATION_MS = 60 * 60 * 1000; // 1 hour
