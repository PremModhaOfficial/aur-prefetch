#!/usr/bin/env python3
"""Build prototype/progress-lab.html: five progress UIs, selection, hillclimb."""
import base64
import html
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "prototype/progress-lab.html"
NOM = ROOT / "prototype/shots/nom-example.png"
W = 88


def top(name, right=""):
    left = "┌ " + name + " "
    tail = (" " + right + " ┐") if right else " ┐"
    return left + "─" * (W + 2 - len(left) - len(tail)) + tail


def ln(s=""):
    if len(s) > W:
        raise SystemExit(f"line too long ({len(s)}): {s}")
    return "│" + s.ljust(W) + "│"


def sep(label=""):
    if not label:
        return "├" + "─" * W + "┤"
    head = "├─ " + label + " "
    return head + "─" * (W + 2 - len(head) - 1) + "┤"


def bottom():
    return "└" + "─" * W + "┘"


def frame(*lines):
    return "\n".join(lines)


CANOPY = frame(
    top("Progress", "3 updates · 00:42"),
    ln(" GATES  ✓ lock  ✓ space 14.2 GiB  ✓ lists 3"),
    sep("TREE"),
    ln(" ▼ 3 packages · 1 running · 1 planned · 1 done · 0 failed · ∑ 00:42"),
    ln("   ◐ linux 6.11.6-1      download 87/142 MiB · 21s · 61%"),
    ln("     └─ blocks: install (all downloads) · mirror archlinux.org"),
    ln("   ○ yay-bin 12.4.1-1    planned · build → package → install"),
    ln("     └─ deps: go ✓, git ✓ · waits for linux download"),
    ln("   ✓ bash 5.2.037-2      done · 1.2 MiB · 2s"),
    ln(""),
    ln(" FOCUS  linux 6.11.6-1 · download · 61% · elapsed 21s"),
    ln(" LOG    23:41:02  downloading linux 87/142 MiB · mirror.example.org"),
    ln("        23:41:04  checking keys"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  esc detach  q quit"),
    bottom(),
)

LANES = frame(
    top("Progress", "3 updates · 00:42"),
    sep("PARALLEL LANES"),
    ln(" DOWNLOAD                  BUILD                    INSTALL"),
    ln(" ◐ linux 87/142 MiB       ⏸ yay-bin (after dl)      ○ after all"),
    ln("   ████████░░░░ 61%        deps: go ✓, git ✓         waits for downloads"),
    ln(" ✓ bash 1.2 MiB · 2s     —                        —"),
    ln(""),
    ln(" ∑ downloads 1 · builds 0 · installs 0 · failed 0 · 00:42"),
    ln(""),
    ln(" CRITICAL PATH  linux download → yay-bin build → install"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  esc detach  q quit"),
    bottom(),
)

TIMELINE = frame(
    top("Progress", "3 updates · 00:42"),
    sep("TIMELINE"),
    ln(" 0s        10s       20s       30s       40s       50s"),
    ln(" bash      ███ 2s"),
    ln(" linux     ███████████████████ 21s"),
    ln(" yay-bin                       ░░░░░░░░░░░?  build after linux + deps"),
    ln(""),
    ln(" history  linux 18s/24s (p50/p90, n=12) · yay-bin 95s/140s (n=6)"),
    ln(" estimate hidden: yay-bin n=6 is below the 10-build threshold"),
    ln(" ∑ 3 packages · 1 running · 1 planned · 0 failed · 00:42"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  esc detach  q quit"),
    bottom(),
)

LEDGER = frame(
    top("Progress", "3 updates · 00:42"),
    sep("LEDGER"),
    ln(" ST  PACKAGE             STAGE      PROGRESS       TIME   BLOCKED BY"),
    ln(" ◐   linux 6.11.6-1      download   87/142 MiB     21s    —"),
    ln(" ○   yay-bin 12.4.1-1    build      waiting        —      linux dl, go, git"),
    ln(" ✓   bash 5.2.037-2      done       1.2 MiB        2s     —"),
    ln(""),
    ln(" ∑ 3 packages · 1 running · 1 planned · 0 failed · 00:42"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  esc detach  q quit"),
    bottom(),
)

SPOTLIGHT = frame(
    top("Progress", "3 updates · 00:42"),
    sep("SPOTLIGHT"),
    ln(" ACTIVE  linux 6.11.6-1 · download"),
    ln(" ████████████████████░░░░  87/142 MiB · 61% · 21s"),
    ln(" mirror.example.org · 4.2 MiB/s measured · no ETA"),
    ln(" last: downloading linux 87/142 MiB"),
    ln(""),
    ln(" QUEUE  ○ yay-bin 12.4.1-1  build · needs go, git"),
    ln("        ✓ bash 5.2.037-2    done · 2s"),
    ln("        – none"),
    ln(" ∑ 1 running · 1 planned · 0 failed · 00:42"),
    sep(),
    ln(" j/k switch  l log  f failures  space pin  esc detach  q quit"),
    bottom(),
)

WINNER = frame(
    top("Progress", "3 updates · 00:42 · Canopy+"),
    ln(" GATES  ✓ lock  ✓ space 14.2 GiB  ✓ lists 3"),
    sep("TREE + CAUSAL EDGES"),
    ln(" ▼ 3 packages · 1 running · 1 planned · 1 done · 0 failed · ∑ 00:42"),
    ln("   ◐ linux 6.11.6-1      download 87/142 MiB · 21s · 61%"),
    ln("     └─ blocks: install (all downloads) · mirror archlinux.org"),
    ln("   ○ yay-bin 12.4.1-1    build · waits for linux, go, git"),
    ln("     └─ buildPhase · p50 95s / p90 140s (n=6, local history)"),
    ln("   ✓ bash 5.2.037-2      done · 1.2 MiB · 2s"),
    ln(""),
    ln(" FOCUS  linux 6.11.6-1 · download · 87/142 MiB · 21s"),
    ln(" LOG    23:41:02  downloading linux 87/142 MiB · mirror.example.org"),
    ln("        23:41:04  checking keys"),
    ln(""),
    ln(" RECEIPT  after run: 3 packages · 143 MiB · 0 failed · log kept"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  a ascii  esc detach  q quit"),
    bottom(),
)

CROSS = frame(
    top("Progress", "gen 1 · Canopy+Ledger · 00:42"),
    sep("TREE"),
    ln(" ▼ 3 packages · 1 running · 1 planned · 1 done · 0 failed · ∑ 00:42"),
    ln("   ◐ linux 6.11.6-1      download 87/142 MiB · 21s"),
    ln("     └─ blocks: install (all downloads)"),
    ln("   ○ yay-bin 12.4.1-1    build · waits for linux, go, git"),
    ln("   ✓ bash 5.2.037-2      done · 1.2 MiB · 2s"),
    sep("LEDGER"),
    ln(" ST  PACKAGE            STAGE     PROGRESS      TIME   BLOCKED BY"),
    ln(" ◐   linux 6.11.6-1     download  87/142 MiB    21s    —"),
    ln(" ○   yay-bin 12.4.1-1   build     waiting       —      linux dl, go, git"),
    ln(" ✓   bash 5.2.037-2     done      1.2 MiB       2s     —"),
    ln(" ∑ 3 packages · 1 running · 1 planned · 0 failed · 00:42"),
    sep(),
    ln(" j/k move  l log  f failures  space focus  esc detach  q quit"),
    bottom(),
)

FAIL = frame(
    top("Progress", "3 updates · 01:12 · failure lens"),
    ln(" TREE"),
    ln(" ▼ 3 packages · 0 running · 1 failed · 2 done · ∑ 01:12"),
    ln("   ✖ yay-bin 12.4.1-1    build failed · exit 4 · 18s"),
    ln("     └─ last: error: target not found: go"),
    ln("     └─ repair: install/provide go; fix depends; do not retry blindly"),
    ln("   ✓ linux 6.11.6-1      done · 142 MiB · 44s"),
    ln("   ✓ bash 5.2.037-2      done · 1.2 MiB · 2s"),
    ln(""),
    ln(" LOG    01:11:58  error: target not found: go"),
    ln("        01:11:58  buildPhase failed · exit 4"),
    ln(""),
    ln(" ∑ 1 failed · repair path shown · pacdiff not run"),
    sep(),
    ln(" l full log  c copy repair  esc close  q quit"),
    bottom(),
)

SCORES = {
    "nom": [10, 10, 9, 8, 9, 3, 4],
    "Canopy": [10, 9, 9, 8, 8, 9, 7],
    "Lanes": [9, 7, 7, 6, 8, 6, 5],
    "Timeline": [8, 6, 10, 7, 6, 7, 5],
    "Ledger": [9, 7, 8, 7, 10, 8, 9],
    "Spotlight": [10, 5, 9, 10, 6, 8, 8],
    "Canopy+Ledger": [10, 9, 9, 8, 9, 9, 8],
    "Spotlight+Canopy": [10, 7, 9, 10, 7, 9, 8],
    "Winner": [10, 10, 10, 10, 10, 10, 9],
}
CRITERIA = ["state", "structure", "timing", "failure", "density", "keyboard", "access"]


def fit(s):
    return round(sum(s) / len(s), 2)


META = [
    dict(id="canopy", score_key="Canopy", name="Canopy", frame=CANOPY,
         idea="nom's tree, but every waiting row says what it is waiting for. A focus pane and a log tail replace the passive redraw.",
         best=["Causal blocked-by edges", "Focus + context", "Keyboard log/failure lens"],
         cost=["Tree can get deep", "Less at-a-glance than a ledger"]),
    dict(id="lanes", score_key="Lanes", name="Lanes", frame=LANES,
         idea="Three parallel lanes: download, build, install. Shows concurrency and the critical path instead of a dependency tree.",
         best=["Parallelism is obvious", "Critical path named", "Good for many builds"],
         cost=["Weak dependency detail", "Lanes waste width when idle"]),
    dict(id="timeline", score_key="Timeline", name="Timeline", frame=TIMELINE,
         idea="A Gantt of the run with p50/p90 history from local builds. The only variant that shows duration and parallelism together.",
         best=["Timing honesty", "History vs estimate is explicit", "Shows where time goes"],
         cost=["No per-item log", "Hard at 80 cols", "Needs history to be useful"]),
    dict(id="ledger", score_key="Ledger", name="Ledger", frame=LEDGER,
         idea="A dense table: state, package, stage, progress, time, blocked-by. Maximum scanability, no tree.",
         best=["Best at 80x24", "Causal column", "Easy to sort"],
         cost=["No dependency shape", "Dense for newcomers"]),
    dict(id="spotlight", score_key="Spotlight", name="Spotlight", frame=SPOTLIGHT,
         idea="One active item large, the queue below. Answers what is happening now; the other variants answer what is happening overall.",
         best=["Best failure focus", "Readable on narrow screens", "Log tail is central"],
         cost=["No overview", "Weak parallelism"]),
    dict(id="winner", score_key="Winner", name="Canopy+", frame=WINNER,
         idea="The selected offspring: Canopy's tree and causal edges, Ledger's dense footer, nom's states/timers/∑, plus failure lens, ASCII and keyboard.",
         best=["Beats nom on causality, control and access", "Failure is a designed state", "Receipt survives the run"],
         cost=["More chrome than nom", "AUR build history is local-only", "Still no ETA when history is thin"]),
]


def table(rows, head):
    return "<table><thead><tr>" + "".join(f"<th>{h}</th>" for h in head) + "</tr></thead><tbody>" + "".join(rows) + "</tbody></table>"


def score_table():
    rows = []
    for name in ["nom", "Canopy", "Lanes", "Timeline", "Ledger", "Spotlight", "Canopy+Ledger", "Spotlight+Canopy", "Winner"]:
        s = SCORES[name]
        cls = " class='winner-row'" if name == "Winner" else ""
        rows.append(f"<tr{cls}><td>{name}</td>" + "".join(f"<td>{x}</td>" for x in s) + f"<td><b>{fit(s):.2f}</b></td></tr>")
    return table(rows, ["variant"] + CRITERIA + ["fitness"])


def hill_table():
    rows = [
        ("M1", "Add causal blocked-by edges to every waiting row", "8.86", "9.00", "+0.14", "kept"),
        ("M2", "Failure lens: last line + repair path + log", "9.00", "9.29", "+0.29", "kept"),
        ("M3", "ASCII/no-colour + linear render-once", "9.29", "9.57", "+0.28", "kept"),
        ("M4", "Keyboard: focus, filter, open log", "9.57", "9.71", "+0.14", "kept"),
        ("M5", "Post-run receipt: bytes, time, next command", "9.71", "9.86", "+0.15", "kept"),
    ]
    return table([f"<tr><td>{a}</td><td>{b}</td><td>{c}</td><td>{d}</td><td>{e}</td><td>{f}</td></tr>" for a, b, c, d, e, f in rows],
                 ["#", "mutation", "before", "after", "delta", "verdict"])


def pane_generations():
    def block(gen, name, fit, frame_text, why):
        return f"""<div class="gen">
  <div class="gen-head"><span class="chip">GEN {gen}</span><b>{name}</b><span class="genfit">{fit}</span></div>
  <pre class="frame">{html.escape(frame_text)}</pre>
  <p>{why}</p>
</div>"""
    rows = [
        ("0", "Canopy, Lanes, Timeline, Ledger, Spotlight", "Canopy", "8.57", "cull Lanes/Timeline; cross Canopy+Ledger"),
        ("1", "Canopy+Ledger, Spotlight+Canopy", "Canopy+Ledger", "8.86", "hillclimb the winner"),
        ("2 · M1", "Canopy+Ledger + causal edges", "Canopy+ M1", "9.00", "add failure lens"),
        ("2 · M2", "M1 + failure lens", "Canopy+ M2", "9.29", "add ASCII/no-colour"),
        ("2 · M3", "M2 + ASCII", "Canopy+ M3", "9.57", "add keyboard"),
        ("2 · M4", "M3 + keyboard", "Canopy+ M4", "9.71", "add receipt"),
        ("2 · M5", "M4 + receipt", "Canopy+ final", "9.86", "stop"),
    ]
    table_rows = "".join(f"<tr><td>{a}</td><td>{b}</td><td>{c}</td><td><b>{d}</b></td><td>{e}</td></tr>" for a, b, c, d, e in rows)
    return f"""
<section class="pane" id="p-generations">
  <div class="kicker">Generations</div>
  <h2>One winner per generation</h2>
  <p class="lede">Gen 0 is the initial population; Gen 1 is the crossover; Gen 2 is the hillclimb. Fitness is the mean of seven criteria. These are self-judged; the two independent scorers are still running and may change the Gen 0/1 ordering.</p>
  {block("0", "Canopy", "8.57", CANOPY, "Canopy beats Lanes, Timeline, Ledger and Spotlight: it keeps the dependency shape and adds the blocked-by reason. Ledger and Spotlight survive as crossover material.")}
  {block("1", "Canopy + Ledger", "8.86", CROSS, "The tree keeps causality; the ledger keeps the full run scannable. This beats Spotlight+Canopy 8.57, so the dense table becomes the footer.")}
  {block("2", "Canopy+ (final)", "9.86", WINNER, "Five kept mutations: causal edges, failure lens, ASCII/no-colour, keyboard, receipt. This is the generation-2 winner; the failure state is shown in the Hillclimb tab.")}
  <div class="tablewrap"><table><thead><tr><th>Gen</th><th>Population</th><th>Winner</th><th>Fitness</th><th>Next</th></tr></thead><tbody>{table_rows}</tbody></table></div>
</section>"""


def main():
    nom_b64 = base64.b64encode(NOM.read_bytes()).decode()
    panes = []
    for m in META:
        chips = "".join(f"<span class='chip'>{html.escape(x)}</span>" for x in m["best"])
        cost = "".join(f"<li>{html.escape(x)}</li>" for x in m["cost"])
        panes.append(f"""
<section class="pane" id="p-{m['id']}">
  <div class="stage">
    <div class="terminal"><div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span>
      <span class="title">aur-prefetch — <b>{html.escape(m['name'])}</b></span><span class="dims">88 × {m['frame'].count(chr(10))+1}</span></div>
      <pre class="frame">{html.escape(m['frame'])}</pre></div>
    <aside class="notes"><div class="kicker">Progress variant</div><h2>{html.escape(m['name'])}</h2>
      <p>{html.escape(m['idea'])}</p><h3>Best at</h3><div class="chips">{chips}</div>
      <h3>Pays for it</h3><ul>{cost}</ul>
      <div class="fit">fitness <b>{fit(SCORES[m['score_key']]):.2f}</b> / 10</div>
    </aside>
  </div>
</section>""")

    panes.append(f"""
<section class="pane" id="p-selection">
  <div class="kicker">Natural selection</div><h2>Five candidates, two offspring, one survivor</h2>
  <p class="lede">Fitness is the mean of seven progress criteria. No variant is scored on the 10-metric shell rubric; this axis is separate. Nom is the baseline to beat. The fitness numbers are a design judgment, not an independent audit; no agent was given a target or minimum.</p>
  <div class="nomref"><img src="data:image/png;base64,{nom_b64}" alt="nix-output-monitor progress screenshot"><div><b>nix-output-monitor</b><p>Tree, states, timers, ∑. Fitness 7.57. Weak on keyboard and accessibility, and it has no repair path.</p></div></div>
  <div class="tablewrap">{score_table()}</div>
  <div class="selection"><div><b>Selected</b><p>Canopy (8.57), Ledger (8.29), Spotlight (8.00). Lanes and Timeline are culled: they win one idea, not the run.</p></div>
  <div><b>Crossover</b><p>Canopy+Ledger 8.86 beats Spotlight+Canopy 8.57. The tree keeps causality; the ledger keeps scanability.</p></div></div>
</section>""")

    panes.append(f"""
<section class="pane" id="p-hillclimb">
  <div class="kicker">Hillclimb</div><h2>One change per iteration on Canopy+</h2>
  <p class="lede">Target: beat nom's 7.57, not a perfect score. Each mutation is kept only if fitness rises and no criterion falls. The final 9.86 still leaves accessibility at 9: ASCII is complete, but a screen reader cannot see the tree shape.</p>
  <div class="tablewrap">{hill_table()}</div>
  <div class="stage" style="margin-top:22px">
    <div class="terminal"><div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span>
      <span class="title">aur-prefetch — <b>Canopy+ failure lens</b></span><span class="dims">88 × {FAIL.count(chr(10))+1}</span></div>
      <pre class="frame">{html.escape(FAIL)}</pre></div>
    <aside class="notes"><div class="kicker">Failure state</div><h2>Repair, not retry</h2>
      <p>nom shows the failed build. Canopy+ shows the last line, the repair command and whether pacdiff ran. That is the gap between a monitor and an operator.</p>
      <h3>Why it beats nom</h3><ul>
        <li>Causal edges say what is blocking, not just the tree shape.</li>
        <li>Keyboard: focus, filter, log, ASCII.</li>
        <li>Failure lens with a repair path and no blind retry.</li>
        <li>Post-run receipt survives the process.</li>
        <li>Timers are measured; ETA is only shown when local history exists.</li>
      </ul><div class="fit">fitness <b>9.86</b> / 10 · nom 7.57</div>
    </aside>
  </div>
</section>""")

    panes.append(pane_generations())
    nav = "".join(f'<button data-target="{m["id"]}">{m["name"]}</button>' for m in META)
    nav += '<button data-target="generations">Generations</button><button data-target="selection">Selection</button><button data-target="hillclimb">Hillclimb</button>'
    doc = f"""<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Progress lab — better than nom?</title><style>{CSS}</style></head><body><div class="wrap">
<div class="kicker">Progress lab · throwaway prototype · no real commands</div>
<h1>Progress, better than nom</h1>
<p class="lede">Question: what should the progress screen look like so it beats <a href="https://github.com/maralorn/nix-output-monitor">nix-output-monitor</a>? Five structural candidates, selected and crossed, then hillclimbed one change at a time.</p>
<div class="warnbox"><b>The bar:</b> nom's tree is excellent at states, structure and timers. It is passive, Nerd-Font/colour dependent, and it does not tell you how to repair a failure. Canopy+ keeps the tree and adds causality, keyboard, ASCII and a failure lens.</div>
<nav id="nav">{nav}</nav>
{''.join(panes)}
</div>
<nav class="switcher"><button id="prev" aria-label="Previous">←</button><span id="label">Canopy</span><button id="next" aria-label="Next">→</button><span class="sepbar"></span><span class="tag">PROTO</span></nav>
<script>
const panes=[...document.querySelectorAll('.pane')];
const buttons=[...document.querySelectorAll('#nav button')];
const names={{canopy:'Canopy',lanes:'Lanes',timeline:'Timeline',ledger:'Ledger',spotlight:'Spotlight',winner:'Canopy+',generations:'Generations',selection:'Selection',hillclimb:'Hillclimb'}};
function show(id){{
  const pane=document.getElementById('p-'+id)||document.getElementById('p-canopy');
  panes.forEach(p=>p.classList.toggle('on',p===pane));
  buttons.forEach(b=>b.classList.toggle('on',b.dataset.target===pane.id.slice(2)));
  document.getElementById('label').textContent=names[pane.id.slice(2)]||pane.id.slice(2);
  try{{const u=new URL(location.href);u.searchParams.set('variant',pane.id.slice(2));history.replaceState(null,'',u)}}catch(e){{}}
}}
buttons.forEach(b=>b.addEventListener('click',()=>show(b.dataset.target)));
function cycle(d){{const ids=Object.keys(names);const cur=ids.findIndex(id=>document.getElementById('p-'+id)?.classList.contains('on'));show(ids[(cur+d+ids.length)%ids.length]);}}
document.getElementById('prev').addEventListener('click',()=>cycle(-1));document.getElementById('next').addEventListener('click',()=>cycle(1));
document.addEventListener('keydown',e=>{{if(e.target.matches('input,textarea,[contenteditable]'))return;if(e.key==='ArrowLeft'){{e.preventDefault();cycle(-1)}}if(e.key==='ArrowRight'){{e.preventDefault();cycle(1)}}}});
show(new URLSearchParams(location.search).get('variant')||'canopy');
</script></body></html>"""
    OUT.write_text(doc)
    print(f"wrote {OUT} ({len(doc)} bytes)")


CSS = """
:root{color-scheme:dark;--bg:#080a0d;--panel:#0e1319;--panel2:#121922;--line:#222b36;--line2:#2c3744;--text:#e2e9f1;--muted:#8c9aab;--dim:#5c6a7a;--accent:#f0a35e;--accent-soft:rgba(240,163,94,.10);--font:ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace}
*{box-sizing:border-box}body{margin:0;background:radial-gradient(1100px 600px at 18% -8%,rgba(52,74,96,.35),transparent 62%),radial-gradient(900px 500px at 92% 0%,rgba(90,64,34,.18),transparent 60%),var(--bg);color:var(--text);font-family:var(--font);font-size:14px;line-height:1.5}
.wrap{max-width:1240px;margin:0 auto;padding:34px 24px 140px}
.kicker{font-size:10px;letter-spacing:.18em;font-weight:700;color:var(--accent);text-transform:uppercase}
h1{font-size:clamp(26px,3.6vw,40px);line-height:1.08;letter-spacing:-.025em;margin:10px 0 12px;font-weight:650}
.lede{color:var(--muted);max-width:860px;font-size:clamp(14px,1.5vw,16px);line-height:1.6;margin:0 0 10px}.lede a{color:var(--accent)}
.warnbox{border:1px solid rgba(240,163,94,.35);background:var(--accent-soft);border-radius:9px;padding:12px 14px;margin:18px 0 22px;font-size:12.5px;line-height:1.6;max-width:940px}.warnbox b{color:var(--accent)}
nav#nav{display:flex;flex-wrap:wrap;gap:6px;margin:22px 0 18px}
nav#nav button{appearance:none;border:1px solid var(--line2);background:var(--panel);color:var(--muted);border-radius:999px;padding:7px 10px;font:600 10.5px/1 var(--font);cursor:pointer}
nav#nav button:hover{color:var(--text);border-color:var(--muted)}nav#nav button.on{color:var(--accent);border-color:rgba(240,163,94,.55);background:var(--accent-soft)}
.pane{display:none}.pane.on{display:block}
.stage{display:grid;grid-template-columns:minmax(0,1fr) 300px;gap:20px;align-items:start}@media(max-width:1040px){.stage{grid-template-columns:1fr}}
.terminal{border:1px solid var(--line);border-radius:12px;background:var(--panel);box-shadow:0 30px 90px rgba(0,0,0,.55);overflow:hidden;min-width:0}
.chrome{display:flex;align-items:center;gap:7px;padding:10px 14px;border-bottom:1px solid var(--line);background:linear-gradient(180deg,var(--panel2),var(--panel))}
.dot{width:8px;height:8px;border-radius:50%;border:1px solid var(--line2)}.dot:nth-child(1){background:rgba(255,123,114,.55)}.dot:nth-child(2){background:rgba(242,198,109,.55)}.dot:nth-child(3){background:rgba(125,220,154,.55)}
.chrome .title{margin-left:8px;font-size:11.5px;color:var(--muted)}.chrome .title b{color:var(--text);font-weight:600}.dims{margin-left:auto;font-size:10px;color:var(--dim)}
pre.frame{margin:0;padding:16px 18px;background:#0a0e13;color:#d3dde8;font-family:var(--font);font-size:clamp(9px,1vw,12px);line-height:1.38;overflow-x:auto;white-space:pre}
.notes{border:1px solid var(--line);background:linear-gradient(180deg,var(--panel2),var(--panel));border-radius:12px;padding:18px;position:sticky;top:20px}
.notes h2{font-size:17px;margin:6px 0 0}.notes p{color:var(--muted);font-size:12.5px;line-height:1.6;margin:10px 0 0}.notes h3{font-size:9px;letter-spacing:.15em;text-transform:uppercase;color:var(--dim);margin:18px 0 7px;font-weight:700}
.chips{display:flex;flex-wrap:wrap;gap:5px}.chip{font-size:9px;border:1px solid rgba(240,163,94,.35);background:var(--accent-soft);color:var(--accent);border-radius:4px;padding:3px 5px}
.notes ul{margin:6px 0 0;padding-left:15px;color:var(--muted);font-size:12px;line-height:1.55}.notes li{margin:4px 0}.notes li::marker{color:var(--accent)}
.fit{border-top:1px solid var(--line);margin-top:16px;padding-top:12px;font-size:11px;color:var(--dim)}.fit b{font-size:20px;color:var(--accent)}
h2{font-size:clamp(22px,2.6vw,30px);margin:8px 0 6px;letter-spacing:-.02em}
.tablewrap{overflow-x:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);box-shadow:0 30px 90px rgba(0,0,0,.4);margin-top:16px}
table{border-collapse:collapse;width:100%;min-width:860px}th,td{padding:8px 9px;border-bottom:1px solid var(--line);text-align:left;font-size:11px}th{background:var(--panel2);color:var(--muted);font-size:9px;letter-spacing:.08em;text-transform:uppercase}td{color:var(--muted)}td:first-child{color:var(--text);font-weight:600}.winner-row td{color:var(--accent)}
.nomref{display:grid;grid-template-columns:minmax(0,320px) 1fr;gap:16px;align-items:center;border:1px solid var(--line);background:var(--panel);border-radius:10px;padding:12px;margin-top:16px}.nomref img{width:100%;border-radius:6px;border:1px solid var(--line2)}.nomref p{color:var(--muted);font-size:12px;margin:4px 0 0}
.selection{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin-top:16px}.selection>div{border:1px solid var(--line);background:var(--panel);border-radius:10px;padding:14px}.selection p{color:var(--muted);font-size:12px;margin:6px 0 0}
.gen{margin-top:22px}.gen-head{display:flex;align-items:center;gap:10px;margin:0 0 8px}.gen-head b{font-size:14px}.genfit{margin-left:auto;color:var(--accent);font-weight:700;font-size:18px}.gen p{color:var(--muted);font-size:12px;margin:8px 0 0;max-width:900px}
.switcher{position:fixed;left:50%;bottom:18px;transform:translateX(-50%);display:flex;align-items:center;gap:6px;padding:7px 9px;background:rgba(14,19,25,.94);border:1px solid var(--line2);border-radius:999px;box-shadow:0 16px 50px rgba(0,0,0,.55);backdrop-filter:blur(14px);z-index:60}
.switcher button{appearance:none;border:1px solid transparent;background:transparent;color:var(--muted);border-radius:999px;padding:7px 10px;font:600 11px/1 var(--font);cursor:pointer}.switcher button:hover{color:var(--text);background:var(--panel2)}
#label{min-width:120px;text-align:center;font-size:11px;color:var(--text)}.sepbar{width:1px;height:18px;background:var(--line2);margin:0 3px}.tag{font-size:8.5px;font-weight:800;letter-spacing:.16em;color:var(--accent);border:1px solid rgba(240,163,94,.45);border-radius:4px;padding:4px 6px}
@media(max-width:620px){.wrap{padding:26px 14px 130px}.nomref{grid-template-columns:1fr}.selection{grid-template-columns:1fr}}
"""

if __name__ == "__main__":
    main()
