<script lang="ts">
  import { BookOpen, Bug } from "lucide-svelte";
  import { reportIssueStore } from "../../stores/modStore";
  import Github from "../GithubIcon.svelte";
  import { formatVersion } from "../../utils/appVersion";
  import { openExternal } from "$lib/opener";
  import {
    APP_REPO_URL,
    MAINTAINER_URL,
    ORIGINAL_AUTHOR_URL,
    WIKI_URL,
  } from "../../constants/links";

  export let version = "";

  const open = async (url: string) => {
    try {
      await openExternal(url);
    } catch (error) {
      console.error("Failed to open URL:", error);
    }
  };
</script>

<div class="container default-scrollbar">
  <div class="about-container">
    <div class="content">
      <h2>About Balatro Mod Manager</h2>

      <div>
        <h3>What is BMM?</h3>
        <p>
          Balatro Mod Manager (BMM) is a tool designed to help you manage and
          install mods for the game Balatro. It provides an easy-to-use
          interface for mod management while maintaining the game's unique
          aesthetic.
        </p>
      </div>

      <div>
        <h3>Features</h3>
        <ul>
          <li>Easy mod installation and management</li>
          <li>Automatic game path detection</li>
          <li>Mod compatibility checking</li>
          <li>Clean, pixel-perfect interface</li>
        </ul>
      </div>

      <div class="button-container">
        <button class="wiki-button" on:click={() => open(WIKI_URL)}>
          <BookOpen size={20} />
          <span>Visit Wiki</span>
        </button>
        <button class="github-button" on:click={() => open(APP_REPO_URL)}>
          <Github size={20} />
          <span>GitHub</span>
        </button>
        <button
          class="report-button"
          on:click={() => reportIssueStore.set({ visible: true })}
        >
          <Bug size={20} />
          <span>Report an issue</span>
        </button>
      </div>

      {#if version}
        <p id="versiontext">Current version: {formatVersion(version)}</p>
      {/if}
    </div>

    <div class="profile-section">
      <div class="app-icon">
        <img src="/images/app-icon.png" alt="" />
      </div>
      <span class="profile-title">
        Maintained by
        <button class="credit-link" on:click={() => open(MAINTAINER_URL)}
          >Radu Ursache</button
        >
      </span>
      <span class="profile-subtitle">
        Originally created by
        <button class="credit-link" on:click={() => open(ORIGINAL_AUTHOR_URL)}
          >Efe (Skyline)</button
        >
      </span>
    </div>
  </div>
</div>

<style>
  .about-container {
    display: flex;
    justify-content: space-between;
    gap: 2rem;
    align-items: center;

    padding: 0rem 2rem;

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

  .profile-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .app-icon {
    flex-shrink: 0;
    width: 160px;
    height: 160px;
  }

  .app-icon img {
    width: 100%;
    height: 100%;
    image-rendering: pixelated;
    filter: drop-shadow(0 0 10px rgba(0, 0, 0, 0.3));
    -webkit-user-drag: none;
    user-select: none;
    -webkit-user-select: none;
    pointer-events: none;
  }

  .content {
    flex: 1;
  }

  .profile-title,
  .profile-subtitle {
    color: #f7f1e4;
    font-size: 1.1rem;
    text-align: center;
    font-family: "M6X11", sans-serif;
    text-shadow:
      -1px -1px 0 #000,
      1px -1px 0 #000,
      -1px 1px 0 #000,
      1px 1px 0 #000;
  }

  .profile-subtitle {
    font-size: 0.9rem;
    margin-top: -0.5rem;
  }

  .credit-link {
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    font: inherit;
    color: #fdcf51;
    text-shadow: inherit;
    cursor: pointer;
  }

  .credit-link:hover {
    color: #fde700;
    text-decoration: underline;
  }

  .wiki-button {
    background-color: #fdcf51;
    border: 4px solid #f7f1e4;
    border-radius: 8px;
    color: #000;
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

  .wiki-button:hover {
    background-color: #fde700;
    transform: scale(1.05);
  }

  .wiki-button:active {
    transform: scale(0.95);
  }

  .github-button {
    background-color: #24292f;
    border: 4px solid #f4eee0;
    border-radius: 8px;
    color: #fff;
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
  }

  .github-button:hover {
    background-color: #32383f;
    transform: scale(1.05);
  }

  .github-button:active {
    transform: scale(0.95);
  }

  .report-button {
    background-color: #56a786;
    border: 4px solid #f4eee0;
    border-radius: 8px;
    color: #fff;
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
  }

  .report-button:hover {
    background-color: #74cca8;
    transform: scale(1.05);
  }

  .report-button:active {
    transform: scale(0.95);
  }

  h2 {
    font-size: 2.5rem;
    margin-bottom: 1rem;
    color: #fdcf51;
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

  #versiontext {
    color: #fde700;
    margin-top: 1rem;
  }

  ul {
    list-style-type: disc;
    margin-left: 1rem;
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

  @media (max-width: 1160px) {
    p {
      font-size: 1rem;
    }
    li {
      font-size: 1rem;
    }

    .app-icon {
      width: 136px;
      height: 136px;
    }
    .profile-title {
      font-size: 0.9rem;
    }
    .profile-subtitle {
      font-size: 0.75rem;
    }

    h2 {
      font-size: 2rem;
    }
    h3 {
      font-size: 1.5rem;
    }
    .wiki-button {
      font-size: 1rem;
      padding: 0.4rem 0.8rem;
    }
    .github-button,
    .report-button {
      font-size: 1rem;
      padding: 0.4rem 0.8rem;
    }
  }
</style>
