# Netease-First Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the primary Explore and Library surfaces a real NetEase-first experience while retaining non-NetEase backend compatibility.

**Architecture:** Explore keeps the existing NetEase recommendation/search stores and removes only the Bilibili/YouTube presentation state, routes, and placeholder shelves. Library uses a string-key tab model (`local`, `favorites`, `downloads`, `netease_playlists`, `netease_albums`) so route queries, active styling, and content branches share one stable contract. Follow-up hardening extends only the NetEase auth/download commands; secondary platform modules remain untouched.

**Tech Stack:** Vue 3 `<script setup>`, TypeScript, Pinia, Vue Router, SCSS, Node-based source regression tests, Vite/Vue type checking.

---

### Task 1: Lock the platform and tab contracts

**Files:**
- Modify: `scripts/test-explore-platform-placeholders.mjs`
- Create: `scripts/test-library-tab-config.mjs`
- Modify: `package.json`

- [x] **Step 1: Write the failing library contract test**

Assert that the five public keys are present, Bilibili/YouTube keys are absent, active state is string-based, and the tab bar uses a five-column responsive grid.

- [x] **Step 2: Run both source tests and confirm RED**

Run `pnpm test:explore-platform-placeholders` and `pnpm test:library-tab-config`; the current three-platform Explore markup and seven-index Library markup must fail for the expected assertions.

### Task 2: Simplify Explore to real NetEase search and discovery

**Files:**
- Modify: `src/views/ExploreView.vue`

- [x] **Step 1: Remove presentation-only platform state and shelves**

Keep `useRouter`, search/player/recommend stores, and `BilibiliCoverImage`; remove route/auth/direct invoke dependencies used only by Bilibili/YouTube discovery, platform tabs, hero cards, and feed loaders.

- [x] **Step 2: Pin search requests to NetEase**

Call `searchStore.search(query, 'netease')` for debounced and repeated searches, and render the existing real NetEase tag/playlist content without a platform selector.

- [x] **Step 3: Run the Explore regression test and typecheck**

Run `pnpm test:explore-platform-placeholders` and `pnpm build` after the Library task is integrated.

### Task 3: Unify the Library segmented switcher

**Files:**
- Modify: `src/views/LibraryView.vue`

- [x] **Step 1: Replace numeric state with a string union**

Map route queries directly to the five valid keys and update header visibility, tab active styling, click handlers, and content branches to use those keys.

- [x] **Step 2: Remove non-NetEase primary tabs and fetches**

Remove Bilibili/YouTube computed values, templates, and mount-time fetches from the primary Library view while retaining NetEase playlist/album loading.

- [x] **Step 3: Make the switcher stable at narrow widths**

Use a five-column grid with equal tracks, bounded labels, and compact mobile spacing so all five choices remain aligned at 575px and below.

- [x] **Step 4: Run the Library contract test**

Run `pnpm test:library-tab-config` and then the full build.

### Task 4: Verify the user-facing shell

**Files:**
- No new production files.

- [x] **Step 1: Run focused tests and build**

Run both contract tests, existing player/search tests, `pnpm build`, and `git diff --check`.

- [x] **Step 2: Inspect the browser proxy at `/explore` and `/library`**

Check desktop and 575×898 viewports; confirm no Bilibili/YouTube hero or tabs are visible, NetEase search/recommendations remain usable, and Library selection remains aligned without overflow.

### Follow-up: Harden the real NetEase account and media flows

- [x] Reject stale search, recommendation, and detail responses.
- [x] Validate account responses with business code `200`, a profile, and a positive user ID.
- [x] Isolate personalized caches across logout and account changes.
- [x] Treat cloud liked IDs as authoritative for NetEase tracks and roll back failed cloud mutations.
- [x] Resolve downloads through a dedicated NetEase command and reject preview-only resources.
- [x] Reset detail-page state across playlist, album, and invalid route transitions.
- [x] Verify focused regressions, the production build, and browser behavior.
