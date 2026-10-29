---
tags: [decision, ci, pages]
date: 2026-10-30
---

# GitHub Pages from site/

GitHub Pages can only serve the repo root or `docs/` from a branch, not a folder named `site/`. So `.github/workflows/pages.yml` uses the GitHub Actions source: it uploads `site/` as the artifact and deploys it, with no build step. The owner sets Settings -> Pages -> Source: GitHub Actions once. The workflow runs on pushes to `main` that touch `site/**` or the workflow file, and by hand.

Related: [[install-script-follows-latest-release]]
