#!/usr/bin/env python3
"""Build prototype/atlas.html: the design the rubric scored, with the checks.

Reads TUI.md for the screen frames and rubric.md for the 50 checks. Writes one
self-contained dossier. Throwaway build tool.
"""
import html
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TUI = (ROOT / "TUI.md").read_text()
RUBRIC = (ROOT / "prototype/hillclimb/rubric.md").read_text()
OUT = ROOT / "prototype/atlas.html"

W = 88


def top(name, right=""):
    left = "┌ " + name + " "
    tail = (" " + right + " ┐") if right else " ┐"
    return left + "─" * (W + 2 - len(left) - len(tail)) + tail


def ln(s=""):
    if len(s) > W:
        raise SystemExit(f"line too long ({len(s)}): {s}")
    return "│" + s.ljust(W) + "│"


def sep():
    return "├" + "─" * W + "┤"


def bottom():
    return "└" + "─" * W + "┘"


def frames_from_markdown(text):
    out = []
    for block in re.findall(r"```[a-z]*\n(.*?)```", text, re.S):
        if "┌" not in block:
            continue
        current = None
        for line in block.split("\n"):
            if line.startswith("┌"):
                if current is not None:
                    out.append("\n".join(current))
                current = [line]
            elif current is not None:
                current.append(line)
                if line.startswith("└"):
                    out.append("\n".join(current))
                    current = None
        if current:
            out.append("\n".join(current))
    return out


def settings_frame():
    L = [
        top("Settings", "read only"),
        ln(" SETTING             VALUE                              SOURCE"),
        ln(" pacman cache        /var/cache/pacman/pkg              default"),
        ln(" yay cache           /home/you/.cache/yay               default"),
        ln(" state file          ~/.local/state/aur-prefetch/...    env AUR_PREFETCH_STATE"),
        ln(" trust file          /etc/aurto/trusted-users           default"),
        ln(" min free            2.0 GiB                            env AUR_PREFETCH_MIN_FREE_BYTES"),
        ln(" theme               ansi                               config"),
        ln(" TUI config          ~/.config/aur-prefetch/tui.conf   not found (optional)"),
        ln(""),
        ln(" This screen is read-only. It writes no config."),
        sep(),
        ln(" y copy value  o copy env name  ? help  q quit"),
        bottom(),
    ]
    return "\n".join(L)


def help_frame():
    L = [
        top("Help"),
        ln(" THE MODEL"),
        ln(" Prefetch downloads. Install changes the system. The gap between them is drift."),
        ln(" Downloading is safe. Installing one package in the gap is a partial upgrade."),
        ln(" Never install singles until the full yay -Syuu has run."),
        ln(""),
        ln(" KEYS"),
        ln(" q quit   ? help   ^p palette   r refresh   R full   i install   F fetch   x clean"),
        ln(" ↑↓ move  / search  f filter  s sort  y copy  space attest  enter run  esc back"),
        ln(""),
        ln(" SAFETY"),
        ln(" Every mutation shows the plan and exact argv. Destructive actions default to Cancel."),
        ln(" The lock is never removed for you. Missing trust needs a second confirm."),
        ln(" News cannot be verified; the attestation is for this session only."),
        sep(),
        ln(" ? close  ↑↓ scroll"),
        bottom(),
    ]
    return "\n".join(L)


RAW = frames_from_markdown(TUI)
if len(RAW) != 10:
    raise SystemExit(f"expected 10 frames, found {len(RAW)}")

SCREENS = [
    dict(id="dashboard", name="Dashboard", frame=RAW[0],
         idea="The first screen answers am I current, what is pending, and what is safe to do next. It also carries the partial-upgrade state the old spec missed.",
         proves=["HIE 1–4", "DEN 3", "STH 1–3", "ACC 1–2"],
         bullets=["Verdict, drift and next action are above the table.",
                  "The partial-upgrade state is explicit, not inferred.",
                  "Disk, timer, lock, trust and cache size share one view."]),
    dict(id="updates", name="Updates", frame=RAW[1],
         idea="A decision table, not a log. The row carries the six facts; the inspector carries the rest and the exact command.",
         proves=["DEN 1–2", "DSC 1", "KEY 2–3"],
         bullets=["Package, repo, installed, available, size and cache state are columns.",
                  "Filter, sort and search are panel-local; fetch is global F.",
                  "No per-row install exists, by design."]),
    dict(id="fetch", name="Fetch", frame=RAW[2],
         idea="The download is a gate checklist plus a nom-inspired progress tree: state glyphs, per-item timers, a ∑ summary and a log tail. No invented percentage.",
         proves=["STH 2", "ACC 4", "ONB 3", "PROG (separate)"],
         bullets=["Lock and space are checked before pacman.",
                  "Planned, running, done and failed are distinct glyphs.",
                  "Causal edges say what a build waits for; the failure lens adds a repair path."]),
    dict(id="install", name="Install gate", frame=RAW[3],
         idea="The one dangerous action gets a pre-flight that mirrors install.rs exactly: user, lock, lists, trust, news. Disk is advice, not a TUI-only gate.",
         proves=["SAF 1–3", "DSC 4", "ONB 4"],
         bullets=["Missing trust needs a second confirm; empty trust refuses.",
                  "News is a session attestation, never a claim that the tool verified it.",
                  "The command is exactly yay -Syuu, as the user."]),
    dict(id="post-success", name="Post-install · success", frame=RAW[4],
         idea="The install is not done when yay exits. Kernel, pacdiff and reboot checks are part of the screen.",
         proves=["SAF 4", "STH 4", "ONB 3"],
         bullets=["Exit code, kernel running vs on disk, pacdiff count and reboot advice.",
                  "p runs sudo pacdiff attached.",
                  "The summary is copyable."]),
    dict(id="post-failure", name="Post-install · failure", frame=RAW[5],
         idea="A failed upgrade is a designed state, not a stack trace. Disk-full mid-install is the dangerous case.",
         proves=["SAF 4", "STH 4", "ONB 2"],
         bullets=["The repair path is pacman -Qkk, not a blind retry.",
                  "Pacdiff is explicitly skipped because the upgrade failed.",
                  "The log is one key away."]),
    dict(id="caches", name="Caches", frame=RAW[6],
         idea="A dry run with columned facts and the exact commands. No invented reclaim total.",
         proves=["DEN 4", "SAF 5", "DSC 4"],
         bullets=["Path, size, policy and action are separate columns.",
                  "Report-only caches are labelled and never touched by apply.",
                  "Apply uses an absolute executable and warns that sudo drops env overrides."]),
    dict(id="doctor", name="Doctor", frame=RAW[7],
         idea="An audit that distinguishes okay, warn, fail and skipped. A missing tool is not a green check.",
         proves=["STH 3", "DSC 4", "ONB 6"],
         bullets=["Each row has a verdict and a fix.",
                  "The trust allowlist is healthy and uses the ok glyph.",
                  "Until doctor ships, the panel says audit unavailable and why."]),
    dict(id="activity", name="Activity", frame=RAW[8],
         idea="The event log for gates, commands, exit codes and verbatim output.",
         proves=["DEN 4", "STH 5", "ACC 5"],
         bullets=["Time, level, source and message are columns.",
                  "Clearing the view writes nothing.",
                  "The log is in memory; no session file."]),
    dict(id="settings", name="Settings", frame=settings_frame(),
         idea="Resolved configuration with provenance. Read-only; it answers which path is actually in use.",
         proves=["DEN 5", "DSC 9", "EXT 4"],
         bullets=["Setting, value and source: default, env or config.",
                  "Unknown config keys warn; invalid values do not crash.",
                  "The TUI never writes the config file."]),
    dict(id="help", name="Help", frame=help_frame(),
         idea="The mental model first, keys second. A help screen that only lists keys has failed.",
         proves=["HIE 5", "DSC 3", "ONB 7"],
         bullets=["Download versus install, and why the gap is drift.",
                  "The partial-upgrade footgun is stated in plain language.",
                  "Contextual keys follow the model."]),
    dict(id="palette", name="Command palette", frame=RAW[9],
         idea="Every action, including disabled ones, with the reason. One registry feeds the palette, footer and help.",
         proves=["DSC 1–2", "KEY 2", "EXT 2"],
         bullets=["Disabled actions remain listed and say why.",
                  "The palette is the discovery path for every panel.",
                  "One action source, so the footer cannot drift."]),
]

STATES = [
    ("empty", "No pending updates. Last fetch 4m ago; last install 9d ago. Still shows disk, timer and trust.", "ONB 1"),
    ("loading", "Keep the last values, dim them, spin only the section header. Never a blank panel.", "ONB 1, STH 2"),
    ("error", "pacman -Qu failed (exit 2): database error. Retry; other sections stay usable.", "STH 1, ONB 5"),
    ("blocked", "pacman is running. Wait for it; the lock is never removed for you.", "ONB 3, SAF 1"),
    ("stale", "Cache sizes measured 6m ago; marked stale after two intervals. Not presented as fresh.", "STH 2"),
    ("unavailable", "AUR updates unavailable: yay not found. Not '0 pending'.", "STH 3, ONB 6"),
]

# Parse the five checks per metric from rubric.md.
RUBRIC_ROWS = []
for line in RUBRIC.splitlines():
    if not line.startswith("| ") or line.startswith("| Metric") or line.startswith("|---"):
        continue
    cells = [c.strip() for c in line.strip("|").split("|")]
    if len(cells) == 6 and cells[0] in {"HIE", "DEN", "CRF", "DSC", "KEY", "SAF", "STH", "ACC", "EXT", "ONB"}:
        RUBRIC_ROWS.append(cells)

if len(RUBRIC_ROWS) != 10:
    raise SystemExit(f"expected 10 rubric rows, found {len(RUBRIC_ROWS)}")

WHERE = {"HIE": "dashboard", "DEN": "updates", "CRF": "dashboard", "DSC": "palette",
         "KEY": "help", "SAF": "install", "STH": "post-success", "ACC": "fetch",
         "EXT": "settings", "ONB": "states"}
NAMES = {"HIE": "Hierarchy", "DEN": "Density", "CRF": "Visual craft", "DSC": "Discoverability",
         "KEY": "Keyboard flow", "SAF": "Safety", "STH": "State honesty",
         "ACC": "Accessibility", "EXT": "Extensibility", "ONB": "Onboarding"}

CSS = """
:root{color-scheme:dark;--bg:#080a0d;--panel:#0e1319;--panel2:#121922;--line:#222b36;--line2:#2c3744;--text:#e2e9f1;--muted:#8c9aab;--dim:#5c6a7a;--accent:#f0a35e;--accent-soft:rgba(240,163,94,.10);--ok:#7ddc9a;--warn:#f2c66d;--font:ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace}
*{box-sizing:border-box}body{margin:0;background:radial-gradient(1100px 600px at 18% -8%,rgba(52,74,96,.35),transparent 62%),radial-gradient(900px 500px at 92% 0%,rgba(90,64,34,.18),transparent 60%),var(--bg);color:var(--text);font-family:var(--font);font-size:14px;line-height:1.5}
.wrap{max-width:1240px;margin:0 auto;padding:34px 24px 120px}
.kicker{font-size:10px;letter-spacing:.18em;font-weight:700;color:var(--accent);text-transform:uppercase}
h1{font-size:clamp(26px,3.6vw,40px);line-height:1.08;letter-spacing:-.025em;margin:10px 0 12px;font-weight:650}
.lede{color:var(--muted);max-width:820px;font-size:clamp(14px,1.5vw,16px);line-height:1.6;margin:0 0 10px}
.warnbox{border:1px solid rgba(240,163,94,.35);background:var(--accent-soft);border-radius:9px;padding:12px 14px;margin:18px 0 22px;color:var(--text);font-size:12.5px;line-height:1.6;max-width:900px}
.warnbox b{color:var(--accent)}
nav{display:flex;flex-wrap:wrap;gap:6px;margin:22px 0 18px}
nav button{appearance:none;border:1px solid var(--line2);background:var(--panel);color:var(--muted);border-radius:999px;padding:7px 10px;font:600 10.5px/1 var(--font);cursor:pointer}
nav button:hover{color:var(--text);border-color:var(--muted)}
nav button.on{color:var(--accent);border-color:rgba(240,163,94,.55);background:var(--accent-soft)}
.pane{display:none}.pane.on{display:block}
.stage{display:grid;grid-template-columns:minmax(0,1fr) 300px;gap:20px;align-items:start}
@media(max-width:1040px){.stage{grid-template-columns:1fr}}
.terminal{border:1px solid var(--line);border-radius:12px;background:var(--panel);box-shadow:0 30px 90px rgba(0,0,0,.55);overflow:hidden;min-width:0}
.chrome{display:flex;align-items:center;gap:7px;padding:10px 14px;border-bottom:1px solid var(--line);background:linear-gradient(180deg,var(--panel2),var(--panel))}
.dot{width:8px;height:8px;border-radius:50%;border:1px solid var(--line2)}
.dot:nth-child(1){background:rgba(255,123,114,.55)}.dot:nth-child(2){background:rgba(242,198,109,.55)}.dot:nth-child(3){background:rgba(125,220,154,.55)}
.chrome .title{margin-left:8px;font-size:11.5px;color:var(--muted)}.chrome .title b{color:var(--text);font-weight:600}
.dims{margin-left:auto;font-size:10px;color:var(--dim);letter-spacing:.06em}
pre.frame{margin:0;padding:16px 18px;background:#0a0e13;color:#d3dde8;font-family:var(--font);font-size:clamp(9px,1.02vw,12.5px);line-height:1.38;overflow-x:auto;white-space:pre}
.notes{border:1px solid var(--line);background:linear-gradient(180deg,var(--panel2),var(--panel));border-radius:12px;padding:18px;position:sticky;top:20px}
.notes h2{font-size:17px;margin:6px 0 0;letter-spacing:-.01em}
.notes p{color:var(--muted);font-size:12.5px;line-height:1.6;margin:10px 0 0}
.notes h3{font-size:9px;letter-spacing:.15em;text-transform:uppercase;color:var(--dim);margin:18px 0 7px;font-weight:700}
.chips{display:flex;flex-wrap:wrap;gap:5px}
.chip{font-size:9px;letter-spacing:.05em;border:1px solid rgba(240,163,94,.35);background:var(--accent-soft);color:var(--accent);border-radius:4px;padding:3px 5px}
.notes ul{margin:6px 0 0;padding-left:15px;color:var(--muted);font-size:12px;line-height:1.55}
.notes li{margin:4px 0}.notes li::marker{color:var(--accent)}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:12px;margin-top:16px}
.card{border:1px solid var(--line);background:var(--panel);border-radius:10px;padding:14px 15px}
.card h3{margin:0 0 6px;font-size:12px;color:var(--text)}
.card p{margin:0;color:var(--muted);font-size:11.5px;line-height:1.55}
.card .chip{display:inline-block;margin-top:9px}
.tablewrap{overflow-x:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);box-shadow:0 30px 90px rgba(0,0,0,.45)}
table{border-collapse:collapse;width:100%;min-width:1020px}
th,td{padding:8px 9px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top;font-size:11px;line-height:1.45}
th{background:var(--panel2);color:var(--muted);font-size:9px;letter-spacing:.09em;text-transform:uppercase;position:sticky;top:0}
td.id{color:var(--accent);font-weight:700;white-space:nowrap}
td.score{color:var(--accent);font-weight:700;font-size:14px;white-space:nowrap}
td.metric{color:var(--text);font-weight:600;white-space:nowrap}
.score-inline{color:var(--accent);font-weight:700;margin-left:7px}
td.check{color:var(--muted);min-width:170px}
td.check:nth-child(2){color:var(--text)}
button.jump{appearance:none;border:0;background:none;color:var(--accent);font:9px/1 var(--font);cursor:pointer;padding:4px 0 0}
.foot{color:var(--dim);font-size:10.5px;line-height:1.6;margin-top:16px;max-width:900px}
code{color:var(--warn)}
"""


def esc(s):
    return html.escape(s)


def pane_screen(s):
    chips = "".join(f'<span class="chip">{esc(c)}</span>' for c in s["proves"])
    bullets = "".join(f"<li>{esc(b)}</li>" for b in s["bullets"])
    height = s["frame"].count("\n") + 1
    return f"""
<section class="pane" id="p-{s['id']}">
  <div class="stage">
    <div class="terminal">
      <div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span>
        <span class="title">Atlas — <b>{esc(s['name'])}</b></span><span class="dims">88 × {height}</span></div>
      <pre class="frame">{esc(s['frame'])}</pre>
    </div>
    <aside class="notes">
      <div class="kicker">Screen</div><h2>{esc(s['name'])}</h2>
      <p>{esc(s['idea'])}</p>
      <h3>Proves</h3><div class="chips">{chips}</div>
      <h3>Why it holds</h3><ul>{bullets}</ul>
    </aside>
  </div>
</section>"""


def pane_states():
    cards = "".join(f"""
    <div class="card"><h3>{esc(name)}</h3><p>{esc(text)}</p><span class="chip">{esc(check)}</span></div>"""
                    for name, text, check in STATES)
    return f"""
<section class="pane" id="p-states">
  <div class="kicker">States</div>
  <h2 style="margin:8px 0 6px;font-size:24px">Every screen has a state for the awkward cases</h2>
  <p class="lede">The rubric asks every screen for empty, loading, error and blocked, plus stale and unavailable where they apply. These are the dashboard's copies; each screen has its own.</p>
  <div class="grid">{cards}</div>
</section>"""


def pane_rubric():
    rows = []
    for metric, core, c2, c3, c4, polish in RUBRIC_ROWS:
        rows.append(
            f"<tr><td class='metric'><b>{esc(metric)}</b> {esc(NAMES[metric])}"
            f"<span class='score-inline'>10.0</span><br>"
            f"<button class='jump' data-jump='{WHERE[metric]}'>show screen</button></td>"
            f"<td class='check'>{esc(core)}</td><td class='check'>{esc(c2)}</td>"
            f"<td class='check'>{esc(c3)}</td><td class='check'>{esc(c4)}</td>"
            f"<td class='check'>{esc(polish)}</td></tr>")
    return f"""
<section class="pane" id="p-rubric">
  <div class="kicker">The 10/10 claim, made inspectable</div>
  <h2 style="margin:8px 0 6px;font-size:24px">Fifty checks, five per metric</h2>
  <p class="lede">The harness claims all fifty pass. Two independent audits verify <b>4.7–6.0 mean, 2.0 floor</b> and flag 20 checks; this table is the claim, not the verdict. Weights are 3 / 2 / 2 / 2 / 1. The baseline vector was HIE 4.0 · DEN 6.0 · CRF 4.0 · DSC 2.0 · KEY 2.0 · SAF 2.0 · STH 0.0 · ACC 0.0 · EXT 0.0 · ONB 2.0.</p>
  <div class="tablewrap"><table><thead><tr><th>Metric</th><th>3 pt · core</th><th>2 pt</th><th>2 pt</th><th>2 pt</th><th>1 pt · polish</th></tr></thead>
  <tbody>{''.join(rows)}</tbody></table></div>
  <p class="foot">What this does not prove: that a person understands the install gate, that the Rust implementation is as clean as the model, or that the terminal renders every glyph in every font. Those need a user test and a running binary. The dossier is the design claim, not the measurement of a product.</p>
</section>"""


def main():
    panes = "\n".join(pane_screen(s) for s in SCREENS)
    nav = "".join(f'<button data-target="{s["id"]}">{esc(s["name"])}</button>' for s in SCREENS)
    nav += '<button data-target="states">States</button><button data-target="rubric">Rubric · 50 checks</button>'
    html_doc = f"""<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Atlas — the claimed 10/10 (dossier)</title><style>{CSS}</style></head><body><div class="wrap">
<div class="kicker">Atlas dossier · claimed 10/10 · independent audit 4.7–6.0</div>
<h1>The claimed 10/10, screen by screen</h1>
<p class="lede">The harness claimed 10/10. Two independent clean-context audits scored the same artifacts at <b>6.0 and 5.1 mean</b> — <b>4.7 conservative, 2.0 floor</b>: HIE 6 · DEN 8 · CRF 5 · DSC 4 · KEY 2 · SAF 3 · STH 6 · ACC 7 · EXT 4 · ONB 2. The dossier shows the design and the gap.</p>
<div class="warnbox"><b>Read this before the score:</b> the 10/10 is a property of <code>design.json</code>, not of these frames. Both auditors found the same failures: screens that start with tables, a palette that omits actions and collides, gates that do not mirror <code>install.rs</code>, no first-run or per-screen state frames, and an ASCII map missing <code>·</code>/<code>…</code>. See <code>prototype/hillclimb/AUDIT.md</code>.</div>
<nav id="nav">{nav}</nav>
{panes}
{pane_states()}
{pane_rubric()}
</div>
<script>
const panes=[...document.querySelectorAll('.pane')];
const buttons=[...document.querySelectorAll('#nav button')];
function show(id){{
  const pane=document.getElementById('p-'+id)||document.getElementById('p-dashboard');
  panes.forEach(p=>p.classList.toggle('on',p===pane));
  buttons.forEach(b=>b.classList.toggle('on',b.dataset.target===pane.id.slice(2)));
  const url=new URL(location.href);url.searchParams.set('screen',pane.id.slice(2));
  try{{history.replaceState(null,'',url)}}catch(e){{}}
  window.scrollTo({{top:0,behavior:'smooth'}});
}}
buttons.forEach(b=>b.addEventListener('click',()=>show(b.dataset.target)));
document.querySelectorAll('[data-jump]').forEach(b=>b.addEventListener('click',()=>show(b.dataset.jump)));
document.addEventListener('keydown',e=>{{
  if(e.target.matches('input,textarea,[contenteditable]'))return;
  const ids=[...document.querySelectorAll('.pane')].map(p=>p.id.slice(2));
  const cur=ids.findIndex(id=>document.getElementById('p-'+id).classList.contains('on'));
  if(e.key==='ArrowRight'){{e.preventDefault();show(ids[(cur+1)%ids.length])}}
  if(e.key==='ArrowLeft'){{e.preventDefault();show(ids[(cur-1+ids.length)%ids.length])}}
}});
show(new URLSearchParams(location.search).get('screen')||'dashboard');
</script></body></html>"""
    OUT.write_text(html_doc)
    print(f"wrote {OUT} ({len(html_doc)} bytes)")


if __name__ == "__main__":
    main()
