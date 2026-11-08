// Hillclimb loop for the Atlas design. One coherent change per iteration,
// measured by the frozen rubric in evaluate.mjs. Keep only if the target metric
// moves up and no metric falls below its previous value.
import fs from "node:fs";
import { score, file } from "./evaluate.mjs";

const read = () => JSON.parse(fs.readFileSync(file("seed.json"), "utf8"));
const write = (d) => fs.writeFileSync(file("design.json"), JSON.stringify(d, null, 2) + "\n");

function merge(a, b) {
  if (Array.isArray(b)) return b.slice();
  if (b && typeof b === "object") {
    const out = a && typeof a === "object" && !Array.isArray(a) ? { ...a } : {};
    for (const [k, v] of Object.entries(b)) out[k] = merge(out[k], v);
    return out;
  }
  return b;
}

const vec = (r) => r.metrics.map((m) => `${m.id}:${m.score.toFixed(1)}`).join(" ");
const metric = (r, id) => r.metrics.find((m) => m.id === id).score;
const checks = (r) => `${r.metrics.reduce((n, m) => n + (5 - m.failed.length), 0)}/50`;

const baseline = read();
let d = structuredClone(baseline);
const baseScore = score(d);

const patches = [
  {
    id: "I01", target: "STH",
    hypothesis: "The report model cannot tell Ready from Failed from NotCollected, so every screen invents its own emptiness.",
    change: "Section enum per domain, per-section timestamps, partial-upgrade drift, slow-section staleness, unknown/unavailable rules, post-install fields, activity argv/exit/gate.",
    patch: {
      report: {
        drift: ["last_fetch", "last_install", "partial_upgrade"],
        sections: {
          official: { state: "Ready", at: true }, aur: { state: "Ready", at: true },
          disk: { state: "Ready", at: true }, lock: { state: "Ready", at: true },
          caches: { state: "Ready", at: true }, timer: { state: "Ready", at: true },
          state: { state: "Ready", at: true }, trust: { state: "Ready", at: true },
          doctor: { state: "NotCollected", at: true }
        },
        slow_sections: true, stale_after: 120,
        unknown_glyph: "?", unavailable_reason: true,
        post_install: ["exit_code", "kernel_running", "kernel_disk", "pacdiff_count", "reboot_required", "news_url"]
      },
      activity_records: ["argv", "exit", "gate"]
    }
  },
  {
    id: "I02", target: "ACC",
    hypothesis: "ASCII mode maps status glyphs only, so borders, gauges, arrows and checkboxes still leak Unicode.",
    change: "Complete ASCII map, colour-never-alone rule, 80x24/120x40 fit, linear --render-once with a fixed clock, reduced motion and absolute timestamps.",
    patch: {
      ascii: { covers_borders: true, covers_gauges: true, covers_arrows: true, covers_checkboxes: true, covers_markers: true },
      color_alone: false,
      responsive: { breakpoints: [80, 120], clipping: false },
      render_once: { linear: true, tty_exempt: true, fixed_clock: true },
      reduced_motion: true,
      absolute_timestamps: true
    }
  },
  {
    id: "I03", target: "EXT",
    hypothesis: "Panels, actions and themes are described but not registered, so each new screen is a fork.",
    change: "Panel registry with a four-step recipe and golden tests, one action source, theme registry, line-based optional config, Section enum.",
    patch: {
      panels: { registry: true, add_steps: 4, golden_tests: true },
      actions_single_source: true,
      theme_registry: true,
      config: { format: "line", optional: true, unknown: "warn", keymap_overrides: true, collision_check: true },
      section_enum: true
    }
  },
  {
    id: "I04", target: "SAF",
    hypothesis: "The TUI adds a disk gate and an attestation the CLI does not have, and only designs the success path.",
    change: "Gates mirror install.rs exactly; missing trust needs a second confirm; empty trust refuses; news is session-only; all post-install cases; clean uses an absolute exe and warns about env loss.",
    patch: {
      gates: ["root", "lock", "official_list", "aur_list", "news", "trust", "yay"],
      trust: { missing: "second_confirm", empty: "refuse" },
      news: { attestation: "session_only" },
      post_install_cases: ["success", "failure", "disk_full", "pacdiff", "kernel", "reboot"],
      clean: { dry_run_default: true, absolute_exe: true, env_warning: true }
    }
  },
  {
    id: "I05", target: "KEY",
    hypothesis: "Global f/c/i collide with panel f/c, and there is no precedence rule or modal keymap.",
    change: "Precedence modal > panel > global; fetch moves to F, clean to x, filter keeps f, clear keeps c; complete list/modal keys; rebindable validated keymap.",
    patch: {
      keymap: {
        precedence: ["modal", "panel", "global"],
        collisions: [],
        bindings: [
          { scope: "global", key: "F", action: "fetch" },
          { scope: "panel", key: "f", action: "filter" },
          { scope: "global", key: "x", action: "clean" },
          { scope: "panel", key: "c", action: "clear" },
          { scope: "global", key: "i", action: "install" }
        ]
      },
      actions: [
        { id: "install", label: "Install", keys: ["i"], danger: "destructive", plan: true, exact_argv: true, confirm: true },
        { id: "fetch", label: "Fetch", keys: ["F"], danger: "mutating", plan: true, exact_argv: true, confirm: true },
        { id: "clean", label: "Clean", keys: ["x"], danger: "destructive", plan: true, exact_argv: true, confirm: true },
        { id: "filter", label: "Filter updates", keys: ["f"], danger: "readonly" },
        { id: "quit", label: "Quit", keys: ["q"], danger: "readonly" }
      ],
      list_nav: ["ArrowUp", "ArrowDown", "j", "k", "PgUp", "PgDn", "g", "G"],
      modal_keys: ["Enter", "Esc", "Tab", "Shift-Tab", "y"]
    }
  },
  {
    id: "I06", target: "DSC",
    hypothesis: "The palette lists actions without reasons, and the footer/help cannot be trusted to match the registry.",
    change: "Every action carries enabled_reason; palette lists all actions; footer and help derive from the registry; empty state names the next command.",
    patch: {
      actions: [
        { id: "install", label: "Install", keys: ["i"], danger: "destructive", plan: true, exact_argv: true, confirm: true, enabled_reason: "blocked by lock/news/trust" },
        { id: "fetch", label: "Fetch", keys: ["F"], danger: "mutating", plan: true, exact_argv: true, confirm: true, enabled_reason: "blocked by lock/space" },
        { id: "clean", label: "Clean", keys: ["x"], danger: "destructive", plan: true, exact_argv: true, confirm: true, enabled_reason: "needs root for apply" },
        { id: "filter", label: "Filter updates", keys: ["f"], danger: "readonly", enabled_reason: "always" },
        { id: "quit", label: "Quit", keys: ["q"], danger: "readonly", enabled_reason: "blocked while a task runs" }
      ],
      palette: { all_actions: true },
      footer: { from_registry: true },
      disabled_reasons: true,
      empty_next_command: true
    }
  },
  {
    id: "I07", target: "ONB",
    hypothesis: "Only some screens define empty/loading/error/blocked, and first run teaches nothing.",
    change: "Every screen declares the four base states; first-run explains the split; install teaches news/trust/reboot; --render-once doubles as a demo.",
    patch: {
      states: ["empty", "loading", "error", "blocked", "stale", "unavailable", "running"],
      screens: {
        dashboard: { states: ["empty", "loading", "error", "blocked", "stale", "unavailable"] },
        updates: { states: ["empty", "loading", "error", "blocked"] },
        install: { states: ["empty", "loading", "error", "blocked"] },
        caches: { states: ["empty", "loading", "error", "blocked"] },
        doctor: { states: ["empty", "loading", "error", "blocked"] },
        activity: { states: ["empty", "loading", "error", "blocked"] },
        settings: { states: ["empty", "loading", "error", "blocked"] },
        help: { states: ["empty", "loading", "error", "blocked"] },
        palette: { states: ["empty", "loading", "error", "blocked"] }
      },
      first_run: true,
      blocked: { gate: true, fix: true, exit_code: true },
      install_teaches: ["news", "trust", "reboot"],
      render_once: { demo: true }
    }
  },
  {
    id: "I08", target: "CRF",
    hypothesis: "Colours are per-screen, truncation is absent, and only one theme exists.",
    change: "One token set, no raw colours, ellipsis truncation, and ansi/dark/light/mono themes.",
    patch: {
      tokens: { fg: "default", muted: "dim", accent: "cyan", ok: "green", warn: "yellow", block: "magenta", error: "red", running: "blue", selected: "reverse" },
      raw_colors: [],
      layout: { border: "single", spacing: 1, truncation: "ellipsis" },
      themes: ["ansi", "dark", "light", "mono"]
    }
  },
  {
    id: "I09", target: "HIE",
    hypothesis: "Only the dashboard starts with a verdict, and the partial-upgrade state is absent from the first screen.",
    change: "Every screen opens with a verdict/summary; exactly one primary action; drift block and partial-upgrade state on the dashboard; help teaches the model.",
    patch: {
      screens: {
        dashboard: { first: "verdict", verdict: ["summary", "drift", "next_action"], blocks: ["verdict", "drift", "pending", "disk", "inodes", "timer", "lock", "trust", "caches"], primary: "install" },
        updates: { first: "verdict", primary: "install" },
        install: { first: "verdict", primary: "install" },
        caches: { first: "verdict", primary: "apply" },
        doctor: { first: "verdict", read_only: true },
        activity: { first: "verdict", read_only: true },
        settings: { first: "verdict", read_only: true },
        help: { first: "summary", summary: true, read_only: true },
        palette: { first: "summary", summary: true, read_only: true }
      },
      report: { drift: ["last_fetch", "last_install", "partial_upgrade"] }
    }
  },
  {
    id: "I10", target: "DEN",
    hypothesis: "Caches and activity hide the columns that make them scannable.",
    change: "Caches gains policy/action, activity gains source, settings keeps setting/value/source.",
    patch: {
      screens: {
        caches: { columns: ["path", "size", "policy", "action"] },
        activity: { columns: ["time", "level", "source", "message"] },
        settings: { columns: ["setting", "value", "source"] }
      }
    }
  }
];

const rows = ["id\thypothesis\tchange\tbefore\tafter\tdelta\ttests\tverdict\tnote"];
let current = score(d);
const accepted = [];

for (const p of patches) {
  const before = current;
  const beforeTarget = metric(before, p.target);
  const candidate = merge(d, p.patch);
  d = candidate;
  write(d);
  const after = score(d);
  const afterTarget = metric(after, p.target);
  const regressed = after.metrics.some((m) => m.score < metric(before, m.id) - 0.001);
  const improved = afterTarget > beforeTarget;
  const keep = improved && !regressed;
  if (!keep) {
    d = structuredClone(baseline);
    for (const q of accepted) d = merge(d, q.patch);
    write(d);
    current = score(d);
  } else {
    accepted.push(p);
    current = after;
  }
  const recorded = keep ? after : current;
  rows.push([
    p.id, p.hypothesis, p.change,
    `min ${before.min.toFixed(1)} (${p.target} ${beforeTarget.toFixed(1)})`,
    `min ${recorded.min.toFixed(1)} (${p.target} ${(keep ? afterTarget : metric(recorded, p.target)).toFixed(1)})`,
    (keep ? afterTarget - beforeTarget : 0).toFixed(1),
    `${checks(recorded)} checks; ${recorded.all ? "PASS" : "below target"}`,
    keep ? "kept" : "reverted",
    keep ? `${p.target} ${beforeTarget.toFixed(1)} -> ${afterTarget.toFixed(1)}` : "no improvement or regression"
  ].join("\t"));
}

fs.writeFileSync(file("decision.tsv"), rows.join("\n") + "\n");
fs.writeFileSync(file("scores.json"), JSON.stringify(current, null, 2) + "\n");

const final = score(d);
console.log("baseline:", vec(baseScore), `min ${baseScore.min.toFixed(1)}`);
for (const m of final.metrics) {
  console.log(`${m.id} ${m.score.toFixed(1)}${m.failed.length ? " FAIL " + m.failed.join(" | ") : ""}`);
}
console.log("final:", vec(final), `min ${final.min.toFixed(1)}`, final.all ? "PASS" : "BELOW TARGET");
console.log("log:", file("decision.tsv"));
process.exit(final.all ? 0 : 1);
