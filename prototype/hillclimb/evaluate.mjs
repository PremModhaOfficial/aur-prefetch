// Frozen rubric harness for the Atlas TUI design.
// 5 checks per metric, weights 3/2/2/2/1 (sum 10). Score is the sum of passed
// weights. Target: every metric >= 9. Run: node evaluate.mjs
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const here = new URL(".", import.meta.url);
export const file = (name) => fileURLToPath(new URL(name, here));

const has = (v) => v !== undefined && v !== null && v !== false && v !== "";
const includesAll = (arr, xs) => Array.isArray(arr) && xs.every((x) => arr.includes(x));
const screens = (d) => d.screens || {};
const everyScreen = (d, fn) => Object.values(screens(d)).every(fn);
const actions = (d) => d.actions || [];

export function score(d) {
  const metrics = [
    {
      id: "HIE", name: "Hierarchy", checks: [
        [3, "dashboard starts with a verdict naming summary, drift and next action",
          () => screens(d).dashboard?.first === "verdict" &&
            includesAll(screens(d).dashboard?.verdict, ["summary", "drift", "next_action"])],
        [2, "every screen starts with a verdict or summary, not a bare table",
          () => everyScreen(d, (s) => s.first === "verdict" || s.first === "summary" || s.summary === true)],
        [2, "every screen has exactly one primary action (or is read-only)",
          () => everyScreen(d, (s) => s.read_only === true || typeof s.primary === "string")],
        [2, "partial-upgrade state is on the drift line and the dashboard",
          () => includesAll(d.report?.drift, ["last_fetch", "last_install", "partial_upgrade"]) &&
            includesAll(screens(d).dashboard?.blocks, ["drift"])],
        [1, "help teaches download/install and the partial-upgrade risk",
          () => includesAll(d.help?.mental_model, ["download", "install", "partial upgrade"])]
      ]
    },
    {
      id: "DEN", name: "Density", checks: [
        [3, "updates is a table with the six decision columns",
          () => includesAll(screens(d).updates?.columns, ["package", "repo", "installed", "available", "size", "cached"])],
        [2, "updates keeps detail in an inspector, not in the row",
          () => includesAll(screens(d).updates?.inspector, ["name", "repo", "cached", "size", "exact_command"])],
        [2, "dashboard shows disk, inodes, timer, lock, trust and caches together",
          () => includesAll(screens(d).dashboard?.blocks, ["disk", "inodes", "timer", "lock", "trust", "caches"])],
        [2, "caches and activity use four-column tables",
          () => includesAll(screens(d).caches?.columns, ["path", "size", "policy", "action"]) &&
            includesAll(screens(d).activity?.columns, ["time", "level", "source", "message"])],
        [1, "settings shows setting, value and source",
          () => includesAll(screens(d).settings?.columns, ["setting", "value", "source"])]
      ]
    },
    {
      id: "CRF", name: "Visual craft", checks: [
        [3, "a full token set exists and screens use no raw colours",
          () => ["fg", "muted", "accent", "ok", "warn", "block", "error", "running", "selected"]
            .every((k) => has(d.tokens?.[k])) && (d.raw_colors || []).length === 0],
        [2, "status glyph semantics are complete and shared",
          () => includesAll(d.glyphs?.semantics, ["ok", "warn", "block", "error", "running", "skipped"])],
        [2, "numeric table columns align right, text left",
          () => d.tables?.numeric_align === "right" && d.tables?.text_align === "left"],
        [2, "one border/spacing grid and ellipsis truncation",
          () => d.layout?.border === "single" && d.layout?.spacing === 1 && d.layout?.truncation === "ellipsis"],
        [1, "ansi, dark, light and mono themes exist",
          () => includesAll(d.themes, ["ansi", "dark", "light", "mono"])]
      ]
    },
    {
      id: "DSC", name: "Discoverability", checks: [
        [3, "every action has label, keys, danger and a reason when disabled",
          () => actions(d).length > 0 && actions(d).every((a) => a.label && a.keys?.length && a.danger && a.enabled_reason) &&
            d.palette?.all_actions === true],
        [2, "footer hints are derived from the action registry",
          () => d.footer?.from_registry === true],
        [2, "help is contextual",
          () => d.help?.contextual === true],
        [2, "disabled actions state why inline",
          () => d.disabled_reasons === true],
        [1, "the empty state names the next command",
          () => d.empty_next_command === true]
      ]
    },
    {
      id: "KEY", name: "Keyboard flow", checks: [
        [3, "a precedence rule exists and no resolved key collides",
          () => includesAll(d.keymap?.precedence, ["modal", "panel", "global"]) &&
            (d.keymap?.collisions || []).length === 0],
        [2, "every action is keyboard-reachable",
          () => actions(d).every((a) => (a.keys && a.keys.length > 0) || a.palette_only === true)],
        [2, "list navigation is complete",
          () => includesAll(d.list_nav, ["ArrowUp", "ArrowDown", "j", "k", "PgUp", "PgDn", "g", "G"])],
        [2, "modal keys are complete",
          () => includesAll(d.modal_keys, ["Enter", "Esc", "Tab", "Shift-Tab", "y"])],
        [1, "keymap is rebindable and collision-validated",
          () => d.config?.keymap_overrides === true && d.config?.collision_check === true]
      ]
    },
    {
      id: "SAF", name: "Safety", checks: [
        [3, "install gates exactly mirror the CLI, no TUI-only blockers",
          () => JSON.stringify(d.gates) ===
            JSON.stringify(["root", "lock", "official_list", "aur_list", "news", "trust", "yay"])],
        [2, "every mutating action shows a plan, exact argv and a confirm",
          () => actions(d).filter((a) => a.danger !== "readonly")
            .every((a) => a.plan === true && a.exact_argv === true && a.confirm === true)],
        [2, "trust missing/empty and news attestation are handled honestly",
          () => d.trust?.missing === "second_confirm" && d.trust?.empty === "refuse" &&
            d.news?.attestation === "session_only"],
        [2, "post-install covers success, failure, disk-full, pacdiff, kernel and reboot",
          () => includesAll(d.post_install_cases, ["success", "failure", "disk_full", "pacdiff", "kernel", "reboot"])],
        [1, "clean is dry-run by default, uses an absolute exe and warns about env loss",
          () => d.clean?.dry_run_default === true && d.clean?.absolute_exe === true &&
            d.clean?.env_warning === true]
      ]
    },
    {
      id: "STH", name: "State honesty", checks: [
        [3, "every report section is Ready/Failed/NotCollected with a timestamp",
          () => Object.values(d.report?.sections || {}).length > 0 &&
            Object.values(d.report.sections).every((s) => ["Ready", "Failed", "NotCollected"].includes(s.state) && s.at === true) &&
            includesAll(d.report?.drift, ["partial_upgrade"])],
        [2, "slow sections expose age and staleness",
          () => d.report?.slow_sections === true && typeof d.report?.stale_after === "number"],
        [2, "unknown is a glyph and unavailable always carries a reason",
          () => d.report?.unknown_glyph === "?" && d.report?.unavailable_reason === true],
        [2, "post-install carries exit code, kernel, pacdiff, reboot and news",
          () => includesAll(d.report?.post_install, ["exit_code", "kernel_running", "kernel_disk", "pacdiff_count", "reboot_required", "news_url"])],
        [1, "activity records argv, exit code and gate",
          () => includesAll(d.activity_records, ["argv", "exit", "gate"])]
      ]
    },
    {
      id: "ACC", name: "Accessibility", checks: [
        [3, "ASCII covers borders, gauges, arrows, checkboxes and markers",
          () => ["borders", "gauges", "arrows", "checkboxes", "markers"]
            .every((k) => d.ascii?.[`covers_${k}`] === true)],
        [2, "colour is never the only carrier of meaning",
          () => d.color_alone === false],
        [2, "80x24 and 120x40 layouts fit without clipping",
          () => includesAll(d.responsive?.breakpoints, [80, 120]) && d.responsive?.clipping === false],
        [2, "render-once is linear, tty-exempt and clock-fixed",
          () => d.render_once?.linear === true && d.render_once?.tty_exempt === true &&
            d.render_once?.fixed_clock === true],
        [1, "reduced motion and absolute timestamps are supported",
          () => d.reduced_motion === true && d.absolute_timestamps === true]
      ]
    },
    {
      id: "EXT", name: "Extensibility", checks: [
        [3, "panel registry, add-a-panel steps and golden tests exist",
          () => d.panels?.registry === true && d.panels?.add_steps >= 4 && d.panels?.golden_tests === true],
        [2, "palette, help and footer derive from one action source",
          () => d.actions_single_source === true],
        [2, "themes are a registry, not per-screen colours",
          () => d.theme_registry === true],
        [2, "config is line-based, optional, warns on unknown keys and validates keymaps",
          () => d.config?.format === "line" && d.config?.optional === true &&
            d.config?.unknown === "warn" && d.config?.keymap_overrides === true &&
            d.config?.collision_check === true],
        [1, "the report uses a Section enum",
          () => d.section_enum === true]
      ]
    },
    {
      id: "ONB", name: "Onboarding", checks: [
        [3, "every screen defines empty, loading, error and blocked states",
          () => everyScreen(d, (s) => includesAll(s.states, ["empty", "loading", "error", "blocked"]))],
        [2, "first run explains the tool and the next command",
          () => d.first_run === true],
        [2, "blocked states name the gate, the fix and the exit code",
          () => d.blocked?.gate === true && d.blocked?.fix === true && d.blocked?.exit_code === true],
        [2, "the install gate teaches news, trust and reboot",
          () => includesAll(d.install_teaches, ["news", "trust", "reboot"])],
        [1, "render-once doubles as a fixture demo",
          () => d.render_once?.demo === true]
      ]
    }
  ];

  const result = metrics.map((m) => {
    let total = 0;
    let score = 0;
    const failed = [];
    for (const [weight, label, fn] of m.checks) {
      total += weight;
      let ok = false;
      try { ok = fn() === true; } catch { ok = false; }
      if (ok) score += weight; else failed.push(label);
    }
    return { id: m.id, name: m.name, score: +(score / total * 10).toFixed(1), failed };
  });

  return {
    name: d.name || "unknown",
    metrics: result,
    min: Math.min(...result.map((m) => m.score)),
    all: result.every((m) => m.score >= 9)
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const d = JSON.parse(fs.readFileSync(file("design.json"), "utf8"));
  const r = score(d);
  for (const m of r.metrics) {
    console.log(`${m.id}  ${m.score.toFixed(1).padStart(4)}  ${m.failed.length ? "FAIL " + m.failed.join(" | ") : "ok"}`);
  }
  console.log(`min ${r.min.toFixed(1)}  ${r.all ? "PASS" : "BELOW TARGET"}`);
  process.exit(r.all ? 0 : 1);
}
