export const APP_REPO = "rursache/balatro-mod-manager";
export const APP_REPO_URL = `https://github.com/${APP_REPO}`;
export const APP_RELEASES_URL = `${APP_REPO_URL}/releases/latest`;
export const APP_LATEST_RELEASE_API = `https://api.github.com/repos/${APP_REPO}/releases/latest`;
export const APP_TAGS_API = `https://api.github.com/repos/${APP_REPO}/tags?per_page=1`;

export const INDEX_REPO = "kasimeka/balatro-mod-index";
export const INDEX_BRANCH = "main";
export const INDEX_REPO_URL = `https://github.com/${INDEX_REPO}`;
export const INDEX_README_URL = `${INDEX_REPO_URL}/blob/${INDEX_BRANCH}/README.md`;
export const INDEX_SCHEMA_URL = `${INDEX_REPO_URL}/blob/${INDEX_BRANCH}/schema/meta.schema.json`;
// GitHub's web editor forks the repo for non members and offers "Propose changes" to open a PR
export const INDEX_NEW_MOD_URL = `${INDEX_REPO_URL}/new/${INDEX_BRANCH}/mods`;
export const INDEX_PULLS_URL = `${INDEX_REPO_URL}/pulls`;
