<script lang="ts">
  import { Upload, ExternalLink, FileCode } from "lucide-svelte";
  import Github from "../GithubIcon.svelte";
  import { openExternal } from "$lib/opener";
  import {
    INDEX_NEW_MOD_URL,
    INDEX_README_URL,
    INDEX_REPO,
    INDEX_REPO_URL,
    INDEX_SCHEMA_URL,
  } from "../../constants/links";

  const open = async (url: string) => {
    try {
      await openExternal(url);
    } catch (error) {
      console.error("Failed to open URL:", error);
    }
  };
</script>

<div class="container default-scrollbar">
  <div class="submit-container">
    <div class="content">
      <div class="title-row">
        <Upload size={32} />
        <h2>Submit a Mod</h2>
      </div>

      <p>
        Want to see your mod in the manager? The catalog comes straight from the
        community mod index on GitHub ({INDEX_REPO}), so adding your mod is a
        pull request to that repository. GitHub's web editor can create the fork
        and the pull request for you.
      </p>

      <div>
        <h3>How it works</h3>
        <ol>
          <li>Read the submission rules and the meta.json schema.</li>
          <li>
            Click "Add Mod Files" (GitHub forks the index for you) or fork it
            yourself.
          </li>
          <li>
            Add <code>mods/Author@ModName/meta.json</code>,
            <code>description.md</code> and optionally
            <code>thumbnail.jpg</code>.
          </li>
          <li>Propose the changes to open a pull request.</li>
          <li>The maintainers review and merge it, then your mod shows up.</li>
        </ol>
      </div>
      <div class="button-container">
        <button
          class="action-button readme-button"
          on:click={() => open(INDEX_README_URL)}
        >
          <ExternalLink size={20} />
          <span>Read Submission Rules</span>
        </button>
        <button
          class="action-button schema-button"
          on:click={() => open(INDEX_SCHEMA_URL)}
        >
          <FileCode size={20} />
          <span>View Schema</span>
        </button>
        <button
          class="action-button submit-button"
          on:click={() => open(INDEX_NEW_MOD_URL)}
        >
          <Upload size={20} />
          <span>Add Mod Files</span>
        </button>
        <button
          class="action-button repo-button"
          on:click={() => open(INDEX_REPO_URL)}
        >
          <Github size={20} />
          <span>View Mod Index Repo</span>
        </button>
      </div>

      <p class="note">Opens in your default browser.</p>
    </div>
  </div>
</div>

<style>
  .submit-container {
    display: flex;
    flex-direction: column;
    padding: 2.5rem 2rem;

    &::-webkit-scrollbar {
      width: 10px;
    }

    &::-webkit-scrollbar-track {
      background: transparent;
      border-radius: 15px;
    }

    &::-webkit-scrollbar-thumb {
      background: #f4eee0;
      border: 2px solid rgba(193, 65, 57, 0.8);
      border-radius: 15px;
    }

    &::-webkit-scrollbar:horizontal {
      display: none;
    }

    &::-webkit-scrollbar-corner {
      background-color: transparent;
    }
  }

  .content {
    flex: 1;
    max-width: 760px;
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    color: #fdcf51;
    margin-bottom: 1rem;
  }

  h2 {
    font-size: 2.5rem;
    color: #fdcf51;
    margin: 0;
  }

  h3 {
    font-size: 1.8rem;
    margin-bottom: 0.5rem;
    color: #fdcf51;
  }

  p {
    font-size: 1.2rem;
    margin-bottom: 1rem;
  }

  .note {
    color: #fde700;
    margin-top: 1rem;
    font-size: 1rem;
  }

  ol {
    list-style-type: decimal;
    margin-left: 1.5rem;
    margin-bottom: 1rem;
  }

  li {
    font-size: 1.2rem;
    margin-bottom: 0.5rem;
  }

  .button-container {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin: 1rem 0;
  }

  .action-button {
    border: 4px solid #f7f1e4;
    border-radius: 8px;
    padding: 0.5rem 1rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.2rem;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 0 10px rgba(0, 0, 0, 0.3);
    margin: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    position: relative;
  }

  .action-button:hover {
    transform: scale(1.05);
  }

  .action-button:active {
    transform: scale(0.95);
  }

  .submit-button {
    background-color: #fdcf51;
    color: #000;
  }

  .submit-button:hover {
    background-color: #fde700;
  }

  .readme-button {
    background-color: #e10600;
    color: #fff;
  }

  .readme-button:hover {
    background-color: #b80500;
  }

  .schema-button {
    background-color: #4a7ec7;
    color: #fff;
  }

  .schema-button:hover {
    background-color: #3c6aab;
  }

  code {
    font-family: monospace;
    font-size: 0.9em;
    color: #fdcf51;
  }

  .repo-button {
    background-color: #24292f;
    color: #fff;
  }

  .repo-button:hover {
    background-color: #32383f;
  }

  @media (max-width: 1160px) {
    p {
      font-size: 1rem;
    }
    li {
      font-size: 1rem;
    }
    h2 {
      font-size: 2rem;
    }
    h3 {
      font-size: 1.5rem;
    }
    .action-button {
      font-size: 1rem;
      padding: 0.4rem 0.8rem;
    }
  }
</style>
