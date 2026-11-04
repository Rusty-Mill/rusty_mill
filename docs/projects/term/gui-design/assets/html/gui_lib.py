"""Shared tokens, CSS and HTML builders for the rusty_term GUI design pages.

Mirrors the Paper file (D5-D30): same tokens, same components. One theme switch
(data-theme="light") re-points the tokens, so every frame renders dark or light.
"""
from __future__ import annotations

from html import escape

FONT_CSS = """
@font-face{font-family:'Instrument Sans';src:url(fonts/instrument-sans-latin-wght-normal.woff2) format('woff2');font-weight:400 700}
@font-face{font-family:'JetBrains Mono';src:url(fonts/jetbrains-mono-latin-wght-normal.woff2) format('woff2');font-weight:100 800}
"""

CSS = FONT_CSS + """
:root{--frame:#171C1F;--base:#1C2226;--raised:#262F34;--sunk:#13171A;--line:#2F393E;
--bone:#DCE2E1;--slate:#8A979C;--dim:#6E7B80;--verd:#5FB8A5;--oxide:#D9773A;
--hover:#2D383E;--press:#364249;--on-verd:#13171A;--cell:#2C363B;--cell-on:#3A464C;
--oxide-bg:rgba(217,119,58,.07);--oxide-bd:rgba(217,119,58,.35);--dots:#3A454A;--win-bd:#323C41;
--shadow:0 8px 24px rgba(0,0,0,.4);--pop-shadow:0 8px 20px rgba(0,0,0,.4);--ui:'Instrument Sans',system-ui,sans-serif;--mono:'JetBrains Mono',ui-monospace,monospace}
[data-theme=light]{--frame:#D3DBDA;--base:#EEF2F1;--raised:#F8FAF9;--sunk:#E1E8E7;--line:#B9C4C3;
--bone:#182024;--slate:#4A585D;--dim:#76848A;--verd:#176B5A;--oxide:#A8460F;
--hover:#E4EAE9;--press:#D6DFDE;--on-verd:#F8FAF9;--cell:#B9C4C3;--cell-on:#9FACAB;
--oxide-bg:rgba(168,70,15,.06);--oxide-bd:rgba(168,70,15,.35);--dots:#B9C4C3;--win-bd:#B9C4C3;
--shadow:0 8px 24px rgba(24,32,36,.14);--pop-shadow:0 8px 20px rgba(24,32,36,.14)}
*{box-sizing:border-box}
body{margin:0;background:#E9EDEC;font-family:var(--ui);color:#182024}
.page{padding:40px;display:flex;flex-direction:column;gap:56px}
.page>h1{font-size:28px;margin:0;font-weight:500}.page>p{margin:-44px 0 0;color:#4A585D;font-size:14px}
.item{display:flex;flex-direction:column;gap:10px}.item>.cap{font-size:13px;font-weight:500;color:#182024}
.item>.note{font-size:12px;color:#4A585D;max-width:1440px}
.row{display:flex;flex-wrap:wrap;gap:40px;align-items:flex-start}
.frame{width:1440px;height:900px;background:#fff;padding:24px;position:relative;overflow:hidden}
.win{position:relative;width:1392px;height:852px;background:var(--frame);border:1px solid var(--win-bd);border-radius:12px;box-shadow:var(--shadow);overflow:hidden;display:flex;flex-direction:column;color:var(--bone);font-size:13px}
.sheet{background:var(--frame);color:var(--bone);padding:40px;width:1440px}
.sheet h2{font-size:18px;margin:0 0 20px;font-weight:500}.sheet .lbl{font-size:12px;color:var(--slate)}
.ic{width:16px;height:16px;fill:none;stroke:currentColor;stroke-width:var(--sw,1.5);stroke-linecap:round;stroke-linejoin:round;flex:none}
.mono{font-family:var(--mono)}
.strip{height:52px;flex:none;display:flex;align-items:center;justify-content:space-between;padding:0 20px 0 22px;position:relative}
.strip .l{display:flex;align-items:center}.dots{display:flex;gap:7px;margin-right:36px}.dots i{width:11px;height:11px;border-radius:50%;background:var(--dots)}
.sess{display:flex;align-items:center;gap:8px}.sess b{font-weight:500}.sess .ic{color:var(--slate)}.sess span{color:var(--slate)}
.vdiv{width:1px;height:24px;background:var(--line);margin:0 16px}
.tabs{display:flex;align-items:center;gap:6px}
.tab{display:flex;align-items:center;gap:10px;height:34px;padding:0 14px 0 12px;border:1px solid transparent;border-radius:7px;color:var(--slate);white-space:pre}
.tab.sel{background:var(--base);border-color:var(--line);color:var(--bone);font-weight:500}
.tab.is-hover{background:var(--hover)}.is-focus{outline:2px solid var(--verd);outline-offset:2px}
.tab .bell{width:6px;height:6px;border-radius:50%;background:var(--bone);margin-left:-2px}
.plus{color:var(--dim);margin-left:6px;display:flex}
.thumb{position:relative;width:26px;height:16px;flex:none}.thumb i{position:absolute;border-radius:1.5px;background:var(--cell)}
.tab.sel .thumb i{background:var(--cell-on)}.thumb i.on{background:var(--verd)!important}
.clients{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--slate)}
.chip{display:inline-flex;align-items:center;gap:8px;height:28px;padding:0 12px;border:1px solid var(--line);border-radius:7px;background:var(--base);font-weight:500;font-size:12px;color:var(--bone)}
.chip .ic{color:var(--slate)}.chip.is-hover{background:var(--hover)}.chip.is-pressed{background:var(--press)}.chip.is-open{background:var(--raised);border-color:var(--verd)}
.blocks{flex:1;display:flex;gap:8px;padding:0 8px 8px;min-height:0;position:relative}
.col{display:flex;flex-direction:column;gap:8px;min-width:0}
.block{position:relative;display:flex;flex-direction:column;background:var(--base);border:1px solid var(--line);border-radius:9px;overflow:hidden;min-height:0}
.block.focus{border:1.5px solid color-mix(in srgb,var(--verd) 70%,transparent)}
.block.dim{opacity:.7}
.bh{height:38px;flex:none;display:flex;align-items:center;justify-content:space-between;padding:0 12px 0 14px;border-bottom:1px solid var(--line)}
.bh .g{display:flex;align-items:center;gap:8px}.bh b{font-weight:500}.bh .s{color:var(--slate)}
.ibtn{display:flex;align-items:center;justify-content:center;width:26px;height:24px;border-radius:5px;background:var(--raised);color:var(--slate)}
.out{padding:12px 16px;font-family:var(--mono);font-size:13px;line-height:21px;color:var(--slate);white-space:pre}
.v{color:var(--verd)}.o{color:var(--oxide)}.b{color:var(--bone)}.sl{color:var(--slate)}.dm{color:var(--dim)}
.badge{position:absolute;left:0;right:0;display:flex;align-items:center;justify-content:center;pointer-events:none}
.badge i{display:flex;align-items:center;justify-content:center;width:32px;height:32px;border-radius:8px;background:var(--raised);border:1px solid var(--line);font-style:normal;font-weight:500;font-size:16px;color:var(--bone)}
.badge i.f{background:var(--verd);border-color:var(--verd);color:var(--on-verd)}
.runs{flex:1;min-height:0;display:flex;flex-direction:column;justify-content:flex-end;gap:8px;padding:0 12px;overflow:hidden}
.run{position:relative;padding:0 14px 8px 18px;border-radius:7px;border:1px solid transparent}
.run:before{content:"";position:absolute;left:-1px;top:8px;bottom:8px;width:3px;border-radius:1.5px;background:var(--verd)}
.run.fail:before{background:var(--oxide)}.run.running:before{background:var(--slate)}
.run.sel{background:var(--oxide-bg);border-color:var(--oxide-bd)}
.run.is-hover{background:var(--hover)}.run.is-focus{outline:2px solid var(--verd);outline-offset:2px}
.rh{display:flex;align-items:center;justify-content:space-between;height:38px}
.rh .cmd{font-family:var(--mono);font-weight:600;font-size:13px;color:var(--bone)}
.rh .st{display:flex;align-items:center;gap:6px;font-size:12px}.rh .st .w{font-weight:500}.rh .st .t{margin-left:8px;color:var(--slate);white-space:pre}
.ok{color:var(--verd)}.bad{color:var(--oxide)}
.rl{font-family:var(--mono);font-size:13px;line-height:21px;color:var(--slate);white-space:pre}
.fold{display:flex;align-items:center;gap:6px;height:21px;margin-left:-2px;font-size:12px;color:var(--slate)}
.rb{display:flex;gap:6px;margin-right:6px}
.inwrap{padding:12px;flex:none}
.input{display:flex;align-items:center;gap:12px;height:50px;padding:0 14px;border-radius:9px;background:var(--sunk);border:1.5px solid var(--line)}
.input.focus{border-color:color-mix(in srgb,var(--verd) 55%,transparent)}.input.disabled{opacity:.45}
.input .ic{color:var(--verd)}.input .t{flex:1;font-family:var(--mono);font-size:14px;white-space:pre;display:flex;align-items:center;min-width:0;overflow:hidden}
.input .hint{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--slate)}
.cur{width:2px;height:19px;background:var(--bone);animation:blink 1.1s steps(1) infinite}.cur.solid{animation:none}
@keyframes blink{50%{opacity:0}}
.ghost{color:var(--dim)}
.kc{display:inline-flex;align-items:center;justify-content:center;height:18px;min-width:18px;padding:0 5px;border:1px solid var(--line);border-radius:4px;font-size:11px;color:var(--slate)}
.kc.act{border-color:var(--verd);color:var(--bone)}.kc.is-pressed{background:var(--press)}.kc.is-hover{background:var(--hover)}
.btn{display:inline-flex;align-items:center;justify-content:center;height:28px;padding:0 12px;border-radius:6px;font-weight:500;font-size:12px;border:1px solid transparent;white-space:nowrap}
.btn.p{background:var(--verd);color:var(--on-verd)}.btn.s{background:var(--raised);border-color:var(--line);color:var(--bone)}.btn.g{color:var(--slate)}
.btn.sm{height:24px;padding:0 10px}
.btn.s.is-hover{background:var(--hover)}.btn.s.is-pressed{background:var(--press)}.btn.g.is-hover{background:var(--hover);color:var(--bone)}.btn.g.is-pressed{background:var(--press);color:var(--bone)}
.btn.p.is-hover{filter:brightness(1.08)}.btn.p.is-pressed{filter:brightness(.92)}
.is-disabled{opacity:.45}
.tag{display:inline-flex;align-items:center;height:20px;padding:0 8px;border:1px solid var(--line);border-radius:5px;font-size:11px;color:var(--slate);background:var(--raised)}
.rib{height:34px;flex:none;display:flex;align-items:center;justify-content:space-between;padding:0 20px 0 24px;background:var(--sunk);border-top:1px solid var(--line);font-size:12px;color:var(--slate)}
.rib .l{display:flex;align-items:center;gap:22px}.rib .dot{width:7px;height:7px;border-radius:50%;background:var(--verd);display:inline-block;margin-right:7px}.rib .dot.off{background:var(--slate)}.rib .dot.bad{background:var(--oxide)}
.rib .r{display:flex;align-items:center;gap:18px}.rib .r>span{display:flex;align-items:center;gap:6px}.rib .r.act{color:var(--bone)}
.pop{position:absolute;background:var(--raised);border:1px solid var(--line);border-radius:9px;box-shadow:var(--pop-shadow);overflow:hidden;z-index:4}
.pop.pad{padding:16px}.pop h4{margin:0 0 6px;font-size:14px;font-weight:500}.pop p{margin:0;font-size:12px;line-height:18px;color:var(--slate)}
.pop .acts{display:flex;justify-content:flex-end;gap:8px;margin-top:12px}
.sw-h{padding:10px 14px 4px;font-size:11px;color:var(--slate)}.sw-h.top{border-top:1px solid var(--line)}
.prow{display:flex;align-items:center;gap:10px;min-height:44px;padding:0 14px 0 17px}
.prow .n{display:flex;flex-direction:column;gap:2px;flex:1}.prow .n b{font-weight:500;font-size:13px}.prow .n span{font-size:12px;color:var(--slate)}
.prow.cur{background:var(--base);border-left:3px solid var(--verd);padding-left:14px}.prow.cur .ok{font-weight:500;font-size:12px}
.prow.is-hover{background:var(--hover)}.prow.is-focus{outline:2px solid var(--verd);outline-offset:-2px}
.pfoot{display:flex;align-items:center;gap:16px;height:34px;padding:0 14px;background:var(--sunk);border-top:1px solid var(--line);font-size:12px;color:var(--slate)}
.pfoot span{display:flex;align-items:center;gap:6px}
.banner{position:absolute;left:0;right:0;top:52px;display:flex;align-items:center;gap:14px;height:44px;padding:0 20px 0 22px;background:var(--raised);border-top:1px solid var(--line);border-bottom:1px solid var(--line);z-index:2}
.banner b{font-weight:500}.banner .d{flex:1;font-size:12px;color:var(--slate)}
.banner.err{border-top:2px solid var(--oxide)}.banner.err .lead{color:var(--oxide)}
.bar{width:200px;height:4px;border-radius:2px;background:var(--sunk)}.bar i{display:block;height:100%;border-radius:2px;background:var(--bone)}
.rail{display:flex;flex-direction:column;gap:8px;width:36px;flex:none}
.rchip{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:3px;height:56px;border:1px solid var(--line);border-radius:9px;background:var(--base);font-weight:500}
.rchip.bad .ic{color:var(--oxide)}
.spin{transform-origin:8px 8px;animation:rot 1s linear infinite}@keyframes rot{to{transform:rotate(360deg)}}
@media (prefers-reduced-motion:reduce){.spin,.cur{animation:none}}
.empty{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;color:var(--slate)}
.empty b{color:var(--bone);font-size:14px;font-weight:500}.empty span{font-size:12px}
.panel{margin:auto;display:flex;flex-direction:column;align-items:flex-start;gap:10px;max-width:420px}
.panel h3{margin:0;font-size:15px;font-weight:500;display:flex;align-items:center;gap:8px}.panel p{margin:0;font-size:12px;line-height:18px;color:var(--slate)}
.panel .acts{display:flex;gap:8px;margin-top:4px}
.mx{display:grid;grid-template-columns:200px repeat(5,1fr);gap:18px 16px;align-items:center}
.mx .h{font-size:12px;color:var(--slate)}.mx .rn{font-size:13px;font-weight:500}
.mx .cell{display:flex;align-items:center;min-height:40px}
.surf{background:var(--base);border:1px solid var(--line);border-radius:9px;padding:14px}
.setwrap{position:absolute;inset:0;z-index:5;display:flex;background:rgba(0,0,0,.45)}
.setpanel{margin:48px auto;width:1000px;height:756px;background:var(--base);border:1px solid var(--line);border-radius:12px;box-shadow:var(--pop-shadow);display:flex;overflow:hidden}
.snav{width:200px;background:var(--sunk);border-right:1px solid var(--line);padding:16px 10px;display:flex;flex-direction:column;gap:2px}
.snav h5{margin:4px 10px 12px;font-size:11px;font-weight:400;color:var(--slate)}
.snav a{display:flex;align-items:center;gap:10px;height:34px;padding:0 10px;border-radius:7px;color:var(--slate);font-size:13px}
.snav a.sel{background:var(--raised);color:var(--bone);font-weight:500}.snav a.is-focus{outline:2px solid var(--verd);outline-offset:-2px}
.snav .foot{margin-top:auto;display:flex;align-items:center;gap:6px;padding:0 10px;font-size:12px;color:var(--slate)}
.scont{flex:1;padding:28px 32px;display:flex;flex-direction:column;gap:6px;overflow:hidden}
.scont h2{margin:0;font-size:20px;font-weight:500}.scont .sub{font-size:12px;color:var(--slate);margin-bottom:14px}
.srow{display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:56px;padding:10px 0;border-bottom:1px solid var(--line)}
.srow .n{display:flex;flex-direction:column;gap:3px;max-width:340px}.srow .n b{font-size:13px;font-weight:500;display:flex;align-items:center;gap:8px}.srow .n span{font-size:12px;line-height:17px;color:var(--slate)}
.srow .ctl{display:flex;align-items:center;gap:8px}
.sfoot{margin-top:auto;display:flex;justify-content:space-between;align-items:center;padding-top:14px}
.seg{display:inline-flex;padding:2px;background:var(--sunk);border:1px solid var(--line);border-radius:8px}
.seg span{display:flex;align-items:center;height:26px;padding:0 12px;border-radius:6px;font-size:12px;color:var(--slate)}
.seg span.on{background:var(--raised);color:var(--bone);font-weight:500;box-shadow:0 0 0 1px var(--line)}
.seg.is-focus{outline:2px solid var(--verd);outline-offset:2px}.seg.is-disabled{opacity:.45}
.tog{width:36px;height:20px;border-radius:10px;background:var(--sunk);border:1px solid var(--line);position:relative;flex:none}
.tog:after{content:"";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--slate)}
.tog.on{background:var(--verd);border-color:var(--verd)}.tog.on:after{left:18px;background:var(--on-verd)}
.tog.is-focus{outline:2px solid var(--verd);outline-offset:2px}.tog.is-disabled{opacity:.45}.tog.is-hover{border-color:var(--slate)}
.sel-c{display:inline-flex;align-items:center;justify-content:space-between;gap:10px;min-width:150px;height:30px;padding:0 10px;border-radius:7px;background:var(--sunk);border:1px solid var(--line);font-size:12px}
.sel-c .ic{color:var(--slate)}.sel-c.is-hover{border-color:var(--slate)}.sel-c.is-focus{outline:2px solid var(--verd);outline-offset:2px}.sel-c.is-disabled{opacity:.45}
.fld{display:inline-flex;align-items:center;height:30px;min-width:170px;padding:0 10px;border-radius:7px;background:var(--sunk);border:1px solid var(--line);font-size:12px}
.fld.mono{font-family:var(--mono)}.fld.is-hover{border-color:var(--slate)}.fld.is-focus{outline:2px solid var(--verd);outline-offset:2px;border-color:var(--verd)}.fld.is-disabled{opacity:.45}
.step{display:inline-flex;align-items:center;border:1px solid var(--line);border-radius:7px;background:var(--sunk);height:30px}
.step span{display:flex;align-items:center;justify-content:center;min-width:34px;height:100%;font-size:12px;padding:0 6px}.step span.val{min-width:64px;border-left:1px solid var(--line);border-right:1px solid var(--line);color:var(--bone)}
.step.is-focus{outline:2px solid var(--verd);outline-offset:2px}.step.is-disabled{opacity:.45}
.ro{display:inline-flex;align-items:center;gap:8px;font-size:12px;color:var(--slate)}.ro .lockv{color:var(--bone);font-family:var(--mono)}
.kedit{display:inline-flex;align-items:center;gap:8px}
.keylist{display:grid;grid-template-columns:repeat(2,1fr);gap:10px 28px;margin-top:6px}
.keylist div{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--slate)}
.hostrow{display:flex;align-items:center;gap:12px;padding:12px 14px;border:1px solid var(--line);border-radius:9px;background:var(--raised);margin-top:8px}
.hostrow .n{flex:1;display:flex;flex-direction:column;gap:2px}.hostrow b{font-size:13px;font-weight:500}.hostrow span{font-size:12px;color:var(--slate)}
.tokst{display:flex;align-items:center;gap:6px;font-size:12px;font-weight:500}
.side{width:240px;flex:none;background:var(--sunk);border-right:1px solid var(--line);display:flex;flex-direction:column;padding:10px 0}
.side.coll{width:44px;align-items:center;gap:6px;padding:10px 0}
.side .sh{display:flex;align-items:center;gap:8px;height:40px;padding:0 14px;font-weight:500}.side .sh span{color:var(--slate);font-weight:400}
.node{display:flex;align-items:center;gap:10px;height:34px;padding:0 14px;color:var(--slate)}.node.sel{background:var(--base);color:var(--bone);font-weight:500;border-left:3px solid var(--verd);padding-left:11px}
.node.child{height:42px;padding-left:44px}.node.child .n{display:flex;flex-direction:column;gap:1px;font-weight:400}.node.child .n b{font-weight:500;color:var(--bone);font-size:12px}.node.child .n span{font-size:11px;color:var(--slate)}
.side .foot{margin-top:auto;padding:12px 14px;display:flex;flex-direction:column;gap:6px;font-size:12px;color:var(--slate);border-top:1px solid var(--line)}
.side .foot div{display:flex;align-items:center;gap:7px}
.rbtn{width:32px;height:32px;display:flex;align-items:center;justify-content:center;border-radius:7px;color:var(--slate);position:relative}.rbtn.sel{background:var(--base);color:var(--bone);border:1px solid var(--line)}
.main{flex:1;display:flex;flex-direction:column;min-width:0}
.hbody{flex:1;display:flex;min-height:0}
.dock{height:36px;flex:none;display:flex;align-items:center;justify-content:space-between;padding:0 14px 0 12px;background:var(--sunk);border-top:1px solid var(--line);font-size:12px;color:var(--slate)}
.dock .l,.dock .r{display:flex;align-items:center;gap:8px}.dock .r{gap:16px}.dock .r>span{display:flex;align-items:center;gap:6px}
.cmp{width:1100px;border-collapse:collapse;font-size:13px;color:var(--bone)}.cmp th,.cmp td{text-align:left;padding:12px 16px;border-bottom:1px solid var(--line);vertical-align:top}.cmp th{font-size:12px;font-weight:500;color:var(--slate)}.cmp td:first-child{font-weight:500;width:200px}
"""

SPRITE = """<svg width="0" height="0" style="position:absolute"><defs>
<symbol id="i-check" viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7"/></symbol>
<symbol id="i-cross" viewBox="0 0 16 16"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/></symbol>
<symbol id="i-branch" viewBox="0 0 16 16"><path d="M5 3v10M5 13a1.8 1.8 0 1 0 0 .1M11 3a1.8 1.8 0 1 0 0 .1M11 4.8c0 4-6 3-6 6.2"/></symbol>
<symbol id="i-chev" viewBox="0 0 16 16"><path d="M6 4l4 4-4 4"/></symbol>
<symbol id="i-chevdown" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4"/></symbol>
<symbol id="i-chevup" viewBox="0 0 16 16"><path d="M4 10l4-4 4 4"/></symbol>
<symbol id="i-prompt" viewBox="0 0 16 16"><path d="M4 4l5 4-5 4"/></symbol>
<symbol id="i-copy" viewBox="0 0 16 16"><path d="M5.5 5.5h7v7h-7zM3.5 10.5v-7h7"/></symbol>
<symbol id="i-rerun" viewBox="0 0 16 16"><path d="M12.5 8a4.5 4.5 0 1 1-1.3-3.2M12.5 3v2.5H10"/></symbol>
<symbol id="i-plus" viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></symbol>
<symbol id="i-folder" viewBox="0 0 16 16"><path d="M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z"/></symbol>
<symbol id="i-server" viewBox="0 0 16 16"><path d="M2.5 3.5h11v4h-11zM2.5 8.5h11v4h-11zM5 5.5h.1M5 10.5h.1"/></symbol>
<symbol id="i-zoom" viewBox="0 0 16 16"><path d="M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3"/></symbol>
<symbol id="i-clients" viewBox="0 0 16 16"><path d="M6 7.5a2.3 2.3 0 1 0 0-.1M2 13c.4-2.4 2-3.6 4-3.6s3.6 1.2 4 3.6M10.5 4.6a2.2 2.2 0 1 1 .8 4.2M12 9.6c1.3.5 2 1.6 2.2 3.4"/></symbol>
<symbol id="i-eye" viewBox="0 0 16 16"><path d="M1.8 8s2.3-4.2 6.2-4.2S14.2 8 14.2 8s-2.3 4.2-6.2 4.2S1.8 8 1.8 8zM8 6.3a1.7 1.7 0 1 0 0 3.4 1.7 1.7 0 0 0 0-3.4z"/></symbol>
<symbol id="i-alert" viewBox="0 0 16 16"><path d="M8 2.5l6 10.5H2zM8 7v3M8 11.5h.1"/></symbol>
<symbol id="i-pause" viewBox="0 0 16 16"><path d="M6 4v8M10 4v8"/></symbol>
<symbol id="i-gear" viewBox="0 0 16 16"><path d="M8 5.8a2.2 2.2 0 1 0 0 4.4 2.2 2.2 0 0 0 0-4.4zM8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1"/></symbol>
<symbol id="i-sidebar" viewBox="0 0 16 16"><path d="M2.5 3.5h11v9h-11zM6.5 3.5v9"/></symbol>
<symbol id="i-lock" viewBox="0 0 16 16"><path d="M4 7.5h8v5.5H4zM5.5 7.5V5.5a2.5 2.5 0 0 1 5 0v2"/></symbol>
</defs></svg>"""


def ic(name: str, color: str | None = None, sw: float | None = None) -> str:
    """One icon set, one stroke (1.5). `sw` is accepted for old call sites and ignored."""
    st = f"color:var(--{color});" if color else ""
    return f'<svg class="ic" style="{st}"><use href="#i-{name}"/></svg>'


def spinner(reduced: bool = False) -> str:
    cls = "" if reduced else "spin"
    arc = "M8 3a5 5 0 0 1 3.5 1.5" if reduced else "M8 3a5 5 0 0 1 5 5"
    return ('<svg class="ic" viewBox="0 0 16 16" ><circle cx="8" cy="8" r="5" style="stroke:var(--line)"/>'
            f'<path class="{cls}" d="{arc}" style="stroke:var(--verd)"/></svg>')


def kc(label: str, state: str = "", act: bool = False) -> str:
    cls = "kc" + (" act" if act else "") + (f" {state}" if state else "")
    return f'<span class="{cls}">{escape(label)}</span>'


def btn(label: str, kind: str = "s", state: str = "", sm: bool = False) -> str:
    return f'<span class="btn {kind}{" sm" if sm else ""}{(" " + state) if state else ""}">{escape(label)}</span>'


def thumb(cells: list[tuple[float, float, float, float, bool]]) -> str:
    out = []
    for x, y, w, h, on in cells:
        out.append(f'<i class="{"on" if on else ""}" style="left:{x*26+.5:.1f}px;top:{y*16+.5:.1f}px;'
                   f'width:{w*26-1.5:.1f}px;height:{h*16-1.5:.1f}px"></i>')
    return f'<span class="thumb">{"".join(out)}</span>'


T_BUILD = [(0, 0, .6, 1, False), (.6, 0, .4, .55, True), (.6, .55, .4, .45, False)]
T_AGENTS = [(0, 0, .5, 1, True), (.5, 0, .5, 1, True)]
T_ONE = [(0, 0, 1, 1, False)]


def tab(n: int, name: str, cells, sel: bool = False, state: str = "", bell: bool = False) -> str:
    cls = "tab" + (" sel" if sel else "") + (f" {state}" if state else "")
    b = '<span class="bell"></span>' if bell else ""
    return f'<span class="{cls}">{thumb(cells)}<span>{n}  {escape(name)}</span>{b}</span>'


def strip(sel: int = 1, detached: int = 0, cells: dict | None = None) -> str:
    cells = cells or {1: T_BUILD, 2: T_AGENTS, 3: T_ONE}
    names = {1: "build", 2: "agents", 3: "baileyai"}
    tabs_html = "".join(tab(i, names[i], cells[i], sel=(i == sel)) for i in (1, 2, 3))
    chip = ""
    if detached:
        chip = f'<span class="chip is-open" style="margin-right:16px">{ic("zoom")}{detached} detached{ic("chevdown")}</span>'
    return (f'<div class="strip"><div class="l"><div class="dots"><i></i><i></i><i></i></div>'
            f'<div class="sess">{ic("server")}<b>rusty_mill</b><span>on baileyai</span>{ic("chevdown")}</div>'
            f'<div class="vdiv"></div><div class="tabs">{tabs_html}<span class="plus">{ic("plus")}</span></div></div>'
            f'<div style="display:flex;align-items:center">{chip}<div class="clients">{ic("clients")}2 clients attached</div></div></div>')


def run(cmd: str, status: str, meta: str, body: str, folded: int = 0, sel: bool = False, state: str = "",
        folded_pos: int = 1) -> str:
    kind = {"Passed": "ok", "Exit 1": "bad", "Running": "running"}.get(status, "ok")
    cls = "run" + (" fail" if kind == "bad" else "") + (" running" if kind == "running" else "") + (" sel" if sel else "") + (f" {state}" if state else "")
    icon = ic("check", "verd", 1.8) if kind == "ok" else ic("cross", "oxide", 1.8) if kind == "bad" else spinner()
    color = "ok" if kind == "ok" else "bad" if kind == "bad" else "sl"
    btns = (f'<span class="rb"><span class="ibtn">{ic("rerun", "bone", 1.4)}</span><span class="ibtn">{ic("copy", "bone", 1.4)}</span></span>'
            if sel else "")
    out = []
    for i, ln in enumerate(body.split("\n")):
        if folded and i == folded_pos:
            out.append(f'<div class="fold">{ic("chev", None, 1.6)}{folded} lines hidden</div>')
        out.append(f'<div class="rl">{ln}</div>')
    return (f'<div class="{cls}"><div class="rh"><span class="cmd">{escape(cmd)}</span>'
            f'<span class="st">{btns}{icon}<span class="w {color}">{status}</span><span class="t">{meta}</span></span></div>{"".join(out)}</div>')


RUN_CLIPPY = run("cargo clippy -p rusty_term -- -D warnings", "Passed", "14.8 s   09:11",
                 '    Checking rusty_term v0.4.0 (crates/libs/ui/rusty_term)\n    <span class="v">Finished</span> dev profile [unoptimized + debuginfo] in 14.71s', folded=212)
RUN_STATUS = run("git status -sb", "Passed", "0.1 s   09:13",
                 '<span class="sl">## </span><span class="v">feat/zoom-text-only</span>...origin/feat/zoom-text-only [behind 1]\n'
                 '<span class="b"> M </span>crates/libs/ui/rusty_term/src/gui/zoom.rs\n<span class="b"> M </span>crates/libs/ui/rusty_term/src/gui/chrome.rs')
RUN_TEST = run("cargo test -p rusty_term", "Passed", "6.2 s   09:14",
               'running 48 tests\ntest result: <span class="v">ok</span>. 48 passed; 0 failed; finished in 6.18s', folded=46)
RUN_PUSH = run("git push origin feat/zoom-text-only", "Exit 1", "0.9 s   09:16",
               'To github.com:baileyrd/rusty_mill.git\n<span class="o"> ! [rejected]  </span><span class="b">feat/zoom-text-only -&gt; feat/zoom-text-only (fetch first)</span>\n'
               '<span class="o">error: </span><span class="b">failed to push some refs to \'github.com:baileyrd/rusty_mill.git\'</span>\n'
               'hint: Updates were rejected because the remote contains work that you\nhint: do not have locally. Integrate the remote changes before pushing.', sel=True)
ALL_RUNS = RUN_CLIPPY + RUN_STATUS + RUN_TEST + RUN_PUSH


def input_box(typed: str = "git pull --re", ghost: str = "base origin feat/zoom-text-only", focus: bool = True,
              disabled: bool = False, solid: bool = False, placeholder: str = "") -> str:
    cls = "input" + (" focus" if focus else "") + (" disabled" if disabled else "")
    cur = f'<span class="cur{" solid" if solid else ""}"></span>' if focus and not disabled else ""
    if typed or ghost:
        text = f'<span class="b">{escape(typed)}</span>{cur}<span class="ghost">{escape(ghost)}</span>'
    else:
        text = f'{cur}<span class="dm">{escape(placeholder)}</span>'
    h = f'<span class="hint">Accept suggestion {kc("Tab")}</span>' if ghost and not disabled else ""
    return f'<div class="inwrap"><div class="{cls}">{ic("prompt", None, 2)}<span class="t">{text}</span>{h}</div></div>'


def shell_header(zoomed: bool = False, cwd: str = "~/src/rusty_mill", branch: str = "feat/zoom-text-only",
                 changes: str = "2 changed, 1 behind") -> str:
    right = (f'<span class="g"><span class="tag">Zoomed</span>{kc("Ctrl-]")}{kc("z")}{btn("Exit zoom", "s", sm=True)}</span>'
             if zoomed else f'<span class="ibtn">{ic("zoom", None, 1.4)}</span>')
    return (f'<div class="bh"><span class="g">{ic("folder", "slate", 1.4)}<b>{cwd}</b>'
            f'{ic("branch", "verd", 1.4)}<span class="b">{branch}</span><span class="s">{changes}</span></span>{right}</div>')


def shell_block(width: str | None = "821px", focus: bool = True, badge: str | None = None, runs: str = ALL_RUNS,
                input_html: str | None = None, hdr: str | None = None, extra: str = "") -> str:
    w = f"width:{width};flex:none;" if width else "flex:1;"
    b = ""
    if badge:
        b = f'<div class="badge" style="top:38px;height:200px"><i class="{"f" if focus else ""}">{badge}</i></div>'
    return (f'<div class="block{" focus" if focus else ""}" style="{w}">{hdr or shell_header()}{b}'
            f'<div class="runs">{runs}</div>{input_html if input_html is not None else input_box()}{extra}</div>')


def agent_header(status: str = "Editing zoom.rs", lease: str = "Agent has input", button: str | None = "Take over",
                 spinning: bool = True, reduced: bool = False) -> str:
    lead = spinner(reduced) if spinning else ic("pause", "slate", 1.8)
    b = btn(button, "s", sm=True) if button else ""
    return (f'<div class="bh"><span class="g">{lead}<b>claude</b><span class="s">{status}</span></span>'
            f'<span class="g" style="gap:10px;font-size:12px">{ic("eye", "slate", 1.3)}<span class="sl" style="margin-left:-4px">{lease}</span>{b}</span></div>')


AGENT_OUT = ('<span class="v">&gt; </span><span class="b">Read </span>src/gui/zoom.rs\n<span class="v">&gt; </span><span class="b">Read </span>src/gui/chrome.rs\n'
             '<span class="v">&gt; </span><span class="b">Edit </span>src/gui/zoom.rs  <span class="v">+12</span> <span class="o">-3</span>\n  ui_px now follows DPI on reset (Ctrl+0)\n'
             '<span class="v">&gt; </span><span class="b">Run  </span>cargo check -p rusty_term\n  Checking rusty_term v0.4.0')


def agent_block(height: str | None = "450px", focus: bool = False, badge: str | None = None, hdr: str | None = None,
                out: str = AGENT_OUT, extra: str = "", dim: bool = False, rail: str = "") -> str:
    h = f"height:{height};flex:none;" if height else "flex:1;"
    b = f'<div class="badge" style="top:38px;bottom:0"><i>{badge}</i></div>' if badge else ""
    return (f'<div class="block{" focus" if focus else ""}{" dim" if dim else ""}" style="{h}">{hdr or agent_header()}{b}'
            f'<div class="out">{out}</div>{extra}</div>')


IDLE_OUT = "[Running 'cargo check']\n    <span class=\"v\">Finished</span> dev profile in 3.84s\n[Finished running. Exit status: 0]"


def idle_block(badge: str | None = None, dim: bool = False, focus: bool = False) -> str:
    b = f'<div class="badge" style="top:38px;bottom:0"><i>{badge}</i></div>' if badge else ""
    return (f'<div class="block{" focus" if focus else ""}{" dim" if dim else ""}" style="flex:1"><div class="bh"><span class="g">{ic("check", "verd", 1.8)}<b>cargo watch -x check</b><span class="s">Idle</span></span></div>{b}'
            f'<div class="out">{IDLE_OUT}</div></div>')


def ribbon(left_state: str = "Connected to baileyai", blocks: str = "3 blocks", dot: str = "", active: bool = False,
           hints: str | None = None) -> str:
    h = hints
    if h is None:
        if active:
            h = (f'<span>{kc("Ctrl-]", act=True)}then</span><span>{kc("h", act=True)}{kc("j", act=True)}{kc("k", act=True)}{kc("l", act=True)}Move focus</span>'
                 f'<span>{kc("1-9", act=True)}Jump</span><span>{kc("|", act=True)}Split right</span><span>{kc("-", act=True)}Split down</span>'
                 f'<span>{kc("z", act=True)}Zoom</span><span>{kc("d", act=True)}Detach</span><span class="sl">{kc("Esc")}to cancel</span>')
        else:
            h = (f'<span>{kc("Ctrl-]")}then</span><span>{kc("|")}Split right</span><span>{kc("-")}Split down</span>'
                 f'<span>{kc("z")}Zoom</span><span>{kc("d")}Detach</span>')
    return (f'<div class="rib"><div class="l"><span><i class="dot {dot}"></i>{left_state}</span><span>{blocks}</span><span>zsh</span><span>UTF-8</span></div>'
            f'<div class="r{" act" if active else ""}">{h}</div></div>')


def default_blocks(shell: str | None = None, agent: str | None = None, idle: str | None = None) -> str:
    return (f'<div class="blocks">{shell or shell_block()}'
            f'<div class="col" style="flex:1">{agent or agent_block()}{idle or idle_block()}</div></div>')


def frame(body: str, theme: str = "dark", overlay: str = "") -> str:
    return f'<section class="frame" data-theme="{theme}"><div class="win">{body}{overlay}</div></section>'


def item(cap: str, content: str, note: str = "") -> str:
    n = f'<div class="note">{note}</div>' if note else ""
    return f'<div class="item"><div class="cap">{cap}</div>{n}{content}</div>'


def page(title: str, sub: str, items: list[str]) -> str:
    return (f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{escape(title)}</title>'
            f'<style>{CSS}</style></head><body>{SPRITE}<div class="page"><h1>{escape(title)}</h1><p>{escape(sub)}</p>'
            f'{"".join(items)}</div>'
            # ?only=N shows just item N (used for screenshots)
            '<script>const q=new URLSearchParams(location.search).get("only");if(q!==null){'
            'document.querySelectorAll(".page>.item").forEach((e,i)=>{if(i!=q)e.style.display="none"});'
            'document.querySelector(".page>h1").style.display="none";document.querySelector(".page>p").style.display="none";'
            'document.querySelector(".page").style.padding="0"}</script></body></html>')
