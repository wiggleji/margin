#!/usr/bin/env bash
# Build a demo repository for screenshots: a small fictional app ("Lumen",
# a tide forecast) with four authors, about three weeks of history, merged
# pull requests, release tags, a hotfix, open feature branches, a local
# "origin" with branches ahead and behind, a stash, uncommitted changes, and
# Margin notes.
#
#   scripts/demo-repo.sh [dir]        (default: ~/margin-demo)
#
# Then open <dir>/lumen in Margin. Dates are relative to now, so the history
# reads "2h ago", "3d ago", … whenever you run it. Works on macOS and Linux.
set -euo pipefail

BASE="${1:-$HOME/margin-demo}"
REPO="$BASE/lumen"
ORIGIN="$BASE/lumen-origin.git"
[ -e "$REPO" ] && { echo "error: $REPO already exists (remove it first)" >&2; exit 1; }
mkdir -p "$BASE"
NOW=$(date +%s)
TZ_OFF="+0900"

git init -q -b main "$REPO"
git init -q --bare -b main "$ORIGIN"
cd "$REPO"
git config commit.gpgsign false
git config tag.gpgsign false
git remote add origin "$ORIGIN"

who() {
  case $1 in
    mina) echo "Mina Park|mina@example.com" ;;
    alex) echo "Alex Chen|alex@example.com" ;;
    sam) echo "Sam Rivera|sam@example.com" ;;
    jo) echo "Jo Kim|jo@example.com" ;;
  esac
}
# as <author> <hours ago> <git args…>: run git as that person at that time
as() {
  local p=$1 h=$2; shift 2
  local id ts; id=$(who "$p"); ts=$((NOW - h * 3600 - (RANDOM % 1200)))
  GIT_AUTHOR_NAME="${id%|*}" GIT_AUTHOR_EMAIL="${id#*|}" \
  GIT_COMMITTER_NAME="${id%|*}" GIT_COMMITTER_EMAIL="${id#*|}" \
  GIT_AUTHOR_DATE="$ts $TZ_OFF" GIT_COMMITTER_DATE="$ts $TZ_OFF" git "$@"
}
commit() { local p=$1 h=$2; shift 2; git add -A; as "$p" "$h" commit -q "$@"; }
merge() { as "$1" "$2" merge -q --no-ff "$3" -m "$4"; }
tag() { as "$1" "$2" tag -a "$3" -m "$4"; }
put() { mkdir -p "$(dirname "$1")"; cat > "$1"; }
branch() { git switch -q -c "$1" "${2:-HEAD}"; }
on() { git switch -q "$1"; }

# ---------------------------------------------------------------- 3 weeks ago
put README.md <<'EOF'
# Lumen

Tide forecasts for the beach you're standing on.

```bash
npm install
npm run dev
```
EOF
put package.json <<'EOF'
{
  "name": "lumen",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "typescript": "^5.6.3",
    "vite": "^5.4.10",
    "vitest": "^2.1.4"
  }
}
EOF
put .gitignore <<'EOF'
node_modules/
dist/
.env
EOF
commit mina 504 -m "Initial commit"

put src/api/client.ts <<'EOF'
export interface TidePoint {
  time: string;
  height: number;
}

const BASE = "https://api.tides.example/v2";

export async function fetchTides(station: string, days = 2): Promise<TidePoint[]> {
  const res = await fetch(`${BASE}/stations/${station}/predictions?days=${days}`);
  if (!res.ok) throw new Error(`tides: ${res.status}`);
  const body = await res.json();
  return body.predictions.map((p: { t: string; v: string }) => ({
    time: p.t,
    height: Number(p.v),
  }));
}
EOF
commit mina 498 -m "Tide API client"

put src/components/TideChart.tsx <<'EOF'
import { TidePoint } from "../api/client";

export function TideChart({ points }: { points: TidePoint[] }) {
  const max = Math.max(...points.map((p) => p.height));
  return (
    <svg viewBox="0 0 100 40" className="tide-chart">
      <polyline
        fill="none"
        points={points.map((p, i) => `${(i / (points.length - 1)) * 100},${40 - (p.height / max) * 36}`).join(" ")}
      />
    </svg>
  );
}
EOF
commit alex 490 -m "Tide chart: first pass"

# feature: units (Alex) + CI on main (Sam)
branch feature/TIDE-12-units
put src/lib/units.ts <<'EOF'
export type Unit = "m" | "ft";

const FT_PER_M = 3.28084;

export function convert(height: number, to: Unit): number {
  return to === "ft" ? height * FT_PER_M : height;
}
EOF
commit alex 470 -m "Units: metres and feet (TIDE-12)"
on main
put .github/workflows/ci.yml <<'EOF'
name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - run: npm ci
      - run: npm test
EOF
commit sam 465 -m "CI: run tests on every push"
on feature/TIDE-12-units
put src/lib/units.ts <<'EOF'
export type Unit = "m" | "ft";

const FT_PER_M = 3.28084;

export function convert(height: number, to: Unit): number {
  return to === "ft" ? height * FT_PER_M : height;
}

export function format(height: number, unit: Unit): string {
  const v = convert(height, unit);
  return `${v.toFixed(unit === "ft" ? 1 : 2)} ${unit}`;
}
EOF
put src/lib/units.test.ts <<'EOF'
import { expect, test } from "vitest";
import { format } from "./units";

test("formats metres and feet", () => {
  expect(format(1.5, "m")).toBe("1.50 m");
  expect(format(1.5, "ft")).toBe("4.9 ft");
});
EOF
commit alex 455 -m "Units: formatting and tests"
on main
merge sam 440 feature/TIDE-12-units "Merge pull request #4 from lumen/feature/TIDE-12-units"
tag mina 436 v0.1.0 "Lumen 0.1.0"

# ---------------------------------------------------------------- 2 weeks ago
branch feature/TIDE-18-tooltips
put src/components/TideChart.tsx <<'EOF'
import { useState } from "react";
import { TidePoint } from "../api/client";
import { format, Unit } from "../lib/units";

export function TideChart({ points, unit }: { points: TidePoint[]; unit: Unit }) {
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(...points.map((p) => p.height));
  const x = (i: number) => (i / (points.length - 1)) * 100;
  const y = (h: number) => 40 - (h / max) * 36;
  return (
    <svg viewBox="0 0 100 40" className="tide-chart" onMouseLeave={() => setHover(null)}>
      <polyline fill="none" points={points.map((p, i) => `${x(i)},${y(p.height)}`).join(" ")} />
      {points.map((p, i) => (
        <rect key={p.time} x={x(i) - 0.5} width={1} height={40} opacity={0} onMouseEnter={() => setHover(i)} />
      ))}
      {hover !== null && (
        <text x={x(hover)} y={y(points[hover].height) - 2} className="tooltip">
          {format(points[hover].height, unit)}
        </text>
      )}
    </svg>
  );
}
EOF
commit mina 420 -m "Chart: tooltip with the height under the pointer (TIDE-18)"

on main
branch fix/TIDE-21-timezone
put src/lib/time.ts <<'EOF'
/** Station times come in UTC; show them in the station's own zone. */
export function localTime(iso: string, zone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    timeZone: zone,
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(iso));
}
EOF
commit jo 410 -m "Show tide times in the station's time zone (TIDE-21)" \
  -m "Times were shown in the browser's zone, so travellers saw high tide hours off."
on feature/TIDE-18-tooltips
put src/styles/chart.css <<'EOF'
.tide-chart polyline {
  stroke: var(--accent);
  stroke-width: 0.6;
}
.tide-chart .tooltip {
  font-size: 3px;
  text-anchor: middle;
  fill: var(--text);
}
EOF
commit mina 400 -m "Chart: tooltip styles"
on fix/TIDE-21-timezone
put src/lib/time.test.ts <<'EOF'
import { expect, test } from "vitest";
import { localTime } from "./time";

test("uses the station zone", () => {
  expect(localTime("2026-06-01T03:00:00Z", "Asia/Seoul")).toMatch(/12:00/);
});
EOF
commit jo 392 -m "Time zone test"
on main
put README.md <<'EOF'
# Lumen

Tide forecasts for the beach you're standing on.

## Develop

```bash
npm install
npm run dev
```

Tests: `npm test`. The API key goes in `.env` as `TIDES_KEY`.
EOF
commit sam 385 -m "Docs: setup steps and the API key"
merge sam 370 fix/TIDE-21-timezone "Merge pull request #7 from lumen/fix/TIDE-21-timezone"
on feature/TIDE-18-tooltips
put src/components/TideChart.tsx <<'EOF'
import { useState } from "react";
import { TidePoint } from "../api/client";
import { format, Unit } from "../lib/units";

export function TideChart({ points, unit }: { points: TidePoint[]; unit: Unit }) {
  const [hover, setHover] = useState<number | null>(null);
  if (points.length < 2) return <p className="empty">No predictions for this station.</p>;
  const max = Math.max(...points.map((p) => p.height));
  const x = (i: number) => (i / (points.length - 1)) * 100;
  const y = (h: number) => 40 - (h / max) * 36;
  return (
    <svg viewBox="0 0 100 40" className="tide-chart" onMouseLeave={() => setHover(null)}>
      <polyline fill="none" points={points.map((p, i) => `${x(i)},${y(p.height)}`).join(" ")} />
      {points.map((p, i) => (
        <rect key={p.time} x={x(i) - 0.5} width={1} height={40} opacity={0} onMouseEnter={() => setHover(i)} />
      ))}
      {hover !== null && (
        <text x={x(hover)} y={y(points[hover].height) - 2} className="tooltip">
          {format(points[hover].height, unit)}
        </text>
      )}
    </svg>
  );
}
EOF
commit mina 360 -m "Chart: message instead of a blank chart when there's no data"
on main
merge alex 350 feature/TIDE-18-tooltips "Merge pull request #6 from lumen/feature/TIDE-18-tooltips"
tag mina 345 v0.2.0 "Lumen 0.2.0"

# hotfix on the release
branch hotfix/TIDE-27-offline v0.2.0
put src/api/client.ts <<'EOF'
export interface TidePoint {
  time: string;
  height: number;
}

const BASE = "https://api.tides.example/v2";

export async function fetchTides(station: string, days = 2): Promise<TidePoint[]> {
  let res: Response;
  try {
    res = await fetch(`${BASE}/stations/${station}/predictions?days=${days}`);
  } catch {
    // offline: keep the last forecast instead of crashing the page
    return JSON.parse(localStorage.getItem(`tides:${station}`) ?? "[]");
  }
  if (!res.ok) throw new Error(`tides: ${res.status}`);
  const body = await res.json();
  const points = body.predictions.map((p: { t: string; v: string }) => ({
    time: p.t,
    height: Number(p.v),
  }));
  localStorage.setItem(`tides:${station}`, JSON.stringify(points));
  return points;
}
EOF
commit jo 330 -m "Fix crash when offline: fall back to the last forecast (TIDE-27)"
on main
merge jo 325 hotfix/TIDE-27-offline "Merge branch 'hotfix/TIDE-27-offline'"
tag jo 324 v0.2.1 "Lumen 0.2.1"
git branch -q -d hotfix/TIDE-27-offline

# ---------------------------------------------------------------- last week
branch feature/TIDE-30-dark-mode
put src/styles/theme.css <<'EOF'
:root {
  --bg: #fbf8f1;
  --text: #2b2f36;
  --accent: #2a7fb8;
}
EOF
commit mina 300 -m "Theme variables (TIDE-30)"
on main
branch feature/TIDE-33-station-search
put src/api/stations.ts <<'EOF'
export interface Station {
  id: string;
  name: string;
  lat: number;
  lon: number;
}

export async function searchStations(q: string): Promise<Station[]> {
  const res = await fetch(`https://api.tides.example/v2/stations?q=${encodeURIComponent(q)}`);
  return (await res.json()).stations;
}
EOF
commit alex 290 -m "Station search API (TIDE-33)"
on feature/TIDE-30-dark-mode
put src/styles/theme.css <<'EOF'
:root {
  --bg: #fbf8f1;
  --text: #2b2f36;
  --accent: #2a7fb8;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1d2128;
    --text: #e6e1d6;
    --accent: #5aaef0;
  }
}
EOF
commit mina 280 -m "Dark mode follows the system setting"
on main
put package.json <<'EOF'
{
  "name": "lumen",
  "version": "0.2.1",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "typescript": "^5.7.2",
    "vite": "^5.4.11",
    "vitest": "^2.1.8"
  }
}
EOF
commit sam 270 -m "Bump dev dependencies"
on feature/TIDE-33-station-search
put src/components/StationSearch.tsx <<'EOF'
import { useEffect, useState } from "react";
import { searchStations, Station } from "../api/stations";

export function StationSearch({ onPick }: { onPick: (s: Station) => void }) {
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<Station[]>([]);
  useEffect(() => {
    if (q.length < 2) return setHits([]);
    const t = setTimeout(() => searchStations(q).then(setHits), 200);
    return () => clearTimeout(t);
  }, [q]);
  return (
    <div className="station-search">
      <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search beaches and harbours" />
      <ul>
        {hits.map((s) => (
          <li key={s.id} onClick={() => onPick(s)}>{s.name}</li>
        ))}
      </ul>
    </div>
  );
}
EOF
commit alex 250 -m "Station search box with debounced lookup"
on feature/TIDE-30-dark-mode
put src/components/ThemeToggle.tsx <<'EOF'
import { useEffect, useState } from "react";

type Theme = "system" | "light" | "dark";

export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(() => (localStorage.getItem("theme") as Theme) ?? "system");
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("theme", theme);
  }, [theme]);
  return (
    <select value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
      <option value="system">System</option>
      <option value="light">Light</option>
      <option value="dark">Dark</option>
    </select>
  );
}
EOF
commit mina 230 -m "Theme picker: system, light, or dark"
on main
merge sam 210 feature/TIDE-30-dark-mode "Merge pull request #11 from lumen/feature/TIDE-30-dark-mode"

branch experiment/canvas-renderer
put src/render/canvas.ts <<'EOF'
import { TidePoint } from "../api/client";

/** Draw on a canvas instead of SVG: a year of data at 60 fps. */
export function drawTides(ctx: CanvasRenderingContext2D, points: TidePoint[]) {
  const { width, height } = ctx.canvas;
  const max = Math.max(...points.map((p) => p.height));
  ctx.clearRect(0, 0, width, height);
  ctx.beginPath();
  points.forEach((p, i) => {
    const x = (i / (points.length - 1)) * width;
    const y = height - (p.height / max) * height * 0.9;
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.stroke();
}
EOF
commit sam 200 -m "Experiment: canvas renderer for long ranges"

on feature/TIDE-33-station-search
put src/styles/search.css <<'EOF'
.station-search input {
  width: 100%;
  padding: 8px 10px;
  border-radius: 6px;
}
.station-search li {
  padding: 6px 10px;
  cursor: pointer;
}
.station-search li:hover {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
EOF
commit alex 180 -m "Search results: hover and spacing"

on main
put src/lib/favourites.ts <<'EOF'
const KEY = "favourites";

export function favourites(): string[] {
  return JSON.parse(localStorage.getItem(KEY) ?? "[]");
}

export function toggleFavourite(id: string): string[] {
  const list = favourites();
  const next = list.includes(id) ? list.filter((s) => s !== id) : [...list, id];
  localStorage.setItem(KEY, JSON.stringify(next));
  return next;
}
EOF
commit jo 160 -m "Favourite stations (TIDE-35)"

# ---------------------------------------------------------------- this week
branch feature/TIDE-38-notifications
put src/notify/schedule.ts <<'EOF'
import { TidePoint } from "../api/client";

/** The next low tide after `now`, if any, for the "go now" reminder. */
export function nextLow(points: TidePoint[], now = Date.now()): TidePoint | undefined {
  for (let i = 1; i < points.length - 1; i++) {
    const p = points[i];
    if (Date.parse(p.time) > now && p.height < points[i - 1].height && p.height <= points[i + 1].height) {
      return p;
    }
  }
}
EOF
commit jo 140 -m "Find the next low tide (TIDE-38)"
on experiment/canvas-renderer
put src/render/canvas.ts <<'EOF'
import { TidePoint } from "../api/client";

/** Draw on a canvas instead of SVG: a year of data at 60 fps. */
export function drawTides(ctx: CanvasRenderingContext2D, points: TidePoint[]) {
  const { width, height } = ctx.canvas;
  const dpr = window.devicePixelRatio || 1;
  const max = Math.max(...points.map((p) => p.height));
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  // one point per pixel column is plenty
  const step = Math.max(1, Math.floor(points.length / width));
  for (let i = 0; i < points.length; i += step) {
    const x = (i / (points.length - 1)) * (width / dpr);
    const y = height / dpr - (points[i].height / max) * (height / dpr) * 0.9;
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  }
  ctx.stroke();
}
EOF
commit sam 120 -m "Canvas: sharp on Retina, skip points smaller than a pixel"
on main
put src/components/TideChart.tsx <<'EOF'
import { memo, useMemo, useState } from "react";
import { TidePoint } from "../api/client";
import { format, Unit } from "../lib/units";

export const TideChart = memo(function TideChart({ points, unit }: { points: TidePoint[]; unit: Unit }) {
  const [hover, setHover] = useState<number | null>(null);
  const max = useMemo(() => Math.max(...points.map((p) => p.height)), [points]);
  if (points.length < 2) return <p className="empty">No predictions for this station.</p>;
  const x = (i: number) => (i / (points.length - 1)) * 100;
  const y = (h: number) => 40 - (h / max) * 36;
  const line = points.map((p, i) => `${x(i)},${y(p.height)}`).join(" ");
  return (
    <svg viewBox="0 0 100 40" className="tide-chart" onMouseLeave={() => setHover(null)}>
      <polyline fill="none" points={line} />
      {points.map((p, i) => (
        <rect key={p.time} x={x(i) - 0.5} width={1} height={40} opacity={0} onMouseEnter={() => setHover(i)} />
      ))}
      {hover !== null && (
        <text x={x(hover)} y={y(points[hover].height) - 2} className="tooltip">
          {format(points[hover].height, unit)}
        </text>
      )}
    </svg>
  );
});
EOF
commit mina 100 -m "Faster chart: memoise the scale and skip re-renders" \
  -m "Hovering re-rendered the whole chart on every mouse move; now only the tooltip changes."
on feature/TIDE-38-notifications
put src/notify/notify.ts <<'EOF'
import { nextLow } from "./schedule";
import { TidePoint } from "../api/client";

export async function remindLowTide(station: string, points: TidePoint[], minutesBefore = 30) {
  if ((await Notification.requestPermission()) !== "granted") return;
  const low = nextLow(points);
  if (!low) return;
  const at = Date.parse(low.time) - minutesBefore * 60_000;
  setTimeout(() => {
    new Notification("Low tide soon", { body: `${station}: low tide in ${minutesBefore} minutes` });
  }, Math.max(0, at - Date.now()));
}
EOF
commit jo 80 -m "Low-tide reminder notification"
on feature/TIDE-33-station-search
put src/api/stations.ts <<'EOF'
export interface Station {
  id: string;
  name: string;
  lat: number;
  lon: number;
}

const cache = new Map<string, Station[]>();

export async function searchStations(q: string): Promise<Station[]> {
  const key = q.trim().toLowerCase();
  const hit = cache.get(key);
  if (hit) return hit;
  const res = await fetch(`https://api.tides.example/v2/stations?q=${encodeURIComponent(key)}`);
  const stations: Station[] = (await res.json()).stations;
  cache.set(key, stations);
  return stations;
}
EOF
commit alex 60 -m "Cache station searches"
on main
put package.json <<'EOF'
{
  "name": "lumen",
  "version": "1.0.0",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "typescript": "^5.7.2",
    "vite": "^5.4.11",
    "vitest": "^2.1.8"
  }
}
EOF
commit sam 50 -m "Release 1.0.0"
tag sam 49 v1.0.0 "Lumen 1.0.0"
on feature/TIDE-38-notifications
put src/notify/schedule.test.ts <<'EOF'
import { expect, test } from "vitest";
import { nextLow } from "./schedule";

test("finds the next low tide", () => {
  const pts = [3, 2, 1, 2, 3].map((height, i) => ({ time: `2030-01-01T0${i}:00:00Z`, height }));
  expect(nextLow(pts, 0)?.height).toBe(1);
});
EOF
commit jo 30 -m "Tests for the low-tide finder"

# ------------------------------------------------- push, then a little more local work
git push -q origin main feature/TIDE-33-station-search feature/TIDE-38-notifications --tags 2>/dev/null
git branch -q -u origin/main main
git branch -q -u origin/feature/TIDE-33-station-search feature/TIDE-33-station-search
git branch -q -u origin/feature/TIDE-38-notifications feature/TIDE-38-notifications

# notifications: one more commit, not pushed yet (↑1)
put src/notify/notify.ts <<'EOF'
import { nextLow } from "./schedule";
import { TidePoint } from "../api/client";

export async function remindLowTide(station: string, points: TidePoint[], minutesBefore = 30) {
  if (!("Notification" in window)) return;
  if ((await Notification.requestPermission()) !== "granted") return;
  const low = nextLow(points);
  if (!low) return;
  const at = Date.parse(low.time) - minutesBefore * 60_000;
  setTimeout(() => {
    new Notification("Low tide soon", { body: `${station}: low tide in ${minutesBefore} minutes`, tag: station });
  }, Math.max(0, at - Date.now()));
}
EOF
commit jo 6 -m "Notifications: skip unsupported browsers, one reminder per station"

# someone else pushed to main (↓1 after fetch)
WORK="$BASE/.lumen-other"
rm -rf "$WORK"
git clone -q "$ORIGIN" "$WORK"
(
  cd "$WORK"
  git config commit.gpgsign false
  put README.md <<'EOF'
# Lumen

Tide forecasts for the beach you're standing on: the next high and low tide,
a chart for the next two days, and a reminder before low tide.

## Develop

```bash
npm install
npm run dev
```

Tests: `npm test`. The API key goes in `.env` as `TIDES_KEY`.
EOF
  git add -A
  as mina 3 commit -q -m "README: what Lumen does"
  git push -q origin main 2>/dev/null
)
rm -rf "$WORK"
git fetch -q origin

# main: a local commit (↑1) and work in progress
on main
put src/lib/units.ts <<'EOF'
export type Unit = "m" | "ft";

const FT_PER_M = 3.28084;

export function convert(height: number, to: Unit): number {
  return to === "ft" ? height * FT_PER_M : height;
}

export function format(height: number, unit: Unit): string {
  const v = convert(height, unit);
  return `${v.toFixed(unit === "ft" ? 1 : 2)} ${unit}`;
}

/** The unit people expect where they are: feet in the US, metres elsewhere. */
export function defaultUnit(locale = navigator.language): Unit {
  return locale.endsWith("-US") ? "ft" : "m";
}
EOF
commit alex 2 -m "Pick feet or metres from the browser's locale"

# a stash
put vite.config.ts <<'EOF'
import { defineConfig } from "vite";

export default defineConfig({ build: { target: "es2022" } });
EOF
git add vite.config.ts
as sam 1 stash push -q -m "WIP: try vite 6 build target"

# uncommitted: one staged change, one unstaged change, one new file
put src/lib/favourites.ts <<'EOF'
const KEY = "favourites";
const MAX = 12;

export function favourites(): string[] {
  return JSON.parse(localStorage.getItem(KEY) ?? "[]");
}

export function toggleFavourite(id: string): string[] {
  const list = favourites();
  const next = list.includes(id) ? list.filter((s) => s !== id) : [...list, id].slice(-MAX);
  localStorage.setItem(KEY, JSON.stringify(next));
  return next;
}
EOF
git add src/lib/favourites.ts
sed -i.bak 's/const y = (h: number) => 40 - (h \/ max) \* 36;/const y = (h: number) => 38 - (h \/ max) * 34; \/\/ room for the tooltip/' src/components/TideChart.tsx
rm -f src/components/TideChart.tsx.bak
put src/lib/format.ts <<'EOF'
export const pct = (n: number) => `${Math.round(n * 100)}%`;
EOF

# ---------------------------------------------------------------- Margin notes
N=".git/margin/notes"
mkdir -p "$N/branch" "$N/commit" "$N/task"
cat > "$N/repo.md" <<'EOF'
# Lumen

- Releases: tag `vX.Y.Z` on main; CI builds and deploys.
- Tide API: 1,000 requests a day on the free plan; cache everything.
- Stations to test with: `9414290` (San Francisco), `KR-0021` (Busan).
EOF
cat > "$N/branch/feature%2FTIDE-38-notifications.md" <<'EOF'
## Low-tide reminders

- [x] find the next low tide
- [x] notification 30 min before
- [ ] let people pick the lead time (15 / 30 / 60 min)
- [ ] Safari needs the page installed to the Dock for notifications
- [ ] re-schedule on `visibilitychange` (a sleeping tab drops timers)
EOF
cat > "$N/branch/feature%2FTIDE-33-station-search.md" <<'EOF'
Waiting on the API team: `/stations?q=` doesn't match accents yet ("Saint-Malo" vs "Saint Malo").
EOF
cat > "$N/commit/$(git rev-parse v1.0.0^{commit}).md" <<'EOF'
## 1.0.0 release

Shipped to production. Announced in #general.

- Watch error rates for the offline fallback (TIDE-27).
- Next: notifications (TIDE-38), station search (TIDE-33).
EOF
cat > "$N/commit/$(git log -1 --format=%H --grep='Faster chart')".md <<'EOF'
Measured on a 2019 MacBook Air: hover went from ~14 ms to ~2 ms per frame.
EOF
cat > "$N/task/TIDE-38.md" <<'EOF'
Design: the reminder copy is "Low tide in 30 minutes at <station>". No sound by default.
EOF

echo "Demo repository ready: $REPO"
echo "  branches: $(git branch --format='%(refname:short)' | tr '\n' ' ')"
echo "  tags:     $(git tag | tr '\n' ' ')"
echo "  open it:  margin \"$REPO\"   (or Open Folder… in Margin)"
