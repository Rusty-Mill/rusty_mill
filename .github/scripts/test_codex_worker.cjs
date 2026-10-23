const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {resolve,supportedPR,sameTarget,safePath,parsePatch,validatePatch,publish} = require('./codex_publication.cjs');
const workflow=fs.readFileSync(path.join(__dirname,'../automation-templates/codex.yml.disabled'),'utf8');
function fixture({sha='head',pr=true,fork=false}={}) {
  const context={repo:{owner:'o',repo:'r'},eventName:'issue_comment',payload:{action:'created',issue:{number:12},comment:{body:'@codex fix',user:{type:'User',login:'human'},author_association:'OWNER'}}};
  const env={WORKER_LOGIN:'verified-fixture-worker'};
  const targetPR={number:30,state:'open',draft:false,user:{login:env.WORKER_LOGIN},head:{ref:'codex/issue-12',sha,repo:{full_name:fork?'fork/r':'o/r'}},base:{ref:'main',sha:'base',repo:{full_name:'o/r'}}};
  const writes=[];
  const github={rest:{users:{getByUsername:async()=>({data:{login:env.WORKER_LOGIN,id:42}})},repos:{getCollaboratorPermissionLevel:async()=>({data:{permission:'write'}}),get:async()=>({data:{default_branch:'main'}})},issues:{get:async()=>({data:{state:'open'}})},git:{getRef:async({ref})=>{
    if(ref==='heads/main')return {data:{object:{sha:'base'}}};
    if(sha)return {data:{object:{sha}}};
    throw Object.assign(new Error('missing'),{status:404});
  },createRef:async x=>writes.push(x)},pulls:{list:()=>{},get:async()=>({data:targetPR}),create:async x=>writes.push(x)}},paginate:async(_fn,args)=>args.state==='open'&&pr?[targetPR]:[],graphql:async(q,vars)=>{
    if(q.includes('viewer'))return {viewer:{login:env.WORKER_LOGIN}};
    writes.push(vars);return {createCommitOnBranch:{commit:{oid:'result'}}};
  }};
  return {github,context,env,targetPR,writes};
}
test('every job disabled, human-created mentions only, no generation write credentials',()=>{
  const jobs=workflow.split(/^  [a-z-]+:\n/gm).slice(1);
  assert.equal(jobs.length,3);
  for(const job of jobs){assert.match(job,/if: \$\{\{ false &&/);assert.match(job,/persist-credentials: false/);assert.match(job,/timeout-minutes:/);}
  assert.match(jobs[0],/comment.user.type == 'User'/);
  assert.match(jobs[0],/contains\(github.event.comment.body, '@codex'\)/);
  assert.match(jobs[1],/permissions: \{contents: read}/);
  assert.match(jobs[1],/permission-profile: ':workspace'/);
  assert.match(jobs[1],/safety-strategy: drop-sudo/);
  assert.doesNotMatch(jobs[1].replace(/^\s*#.*$/gm,''),/GH_TOKEN|GITHUB_TOKEN|PAT|WORKER_LOGIN|: write|token:/);
  assert.doesNotMatch(workflow,/yourname|pull_request_target|force|allow-bots|allowed_bots/);
  assert.match(workflow,/queue: max/);
  assert.match(workflow,/group: codex-worker-\$\{\{ github.repository }}/);
  assert.match(workflow,/cancel-in-progress: false/);
  assert.match(jobs[2],/path: automation/);
  for(const use of workflow.matchAll(/uses: ([^\s]+)/g))assert.match(use[1],/@[a-f0-9]{40}$/);
});
test('repeat issue requests resume existing implementation PR exact head',async()=>{
  const f=fixture();const target=await resolve(f);
  assert.equal(target.sha,'head');assert.equal(target.branch,'codex/issue-12');assert.equal(target.pr,30);assert.equal(target.newBranch,false);assert.equal(f.writes.length,0);
});
test('new issue uses default only when no previous branch or PR exists',async()=>{
  const f=fixture({sha:null,pr:false});const target=await resolve(f);
  assert.equal(target.sha,'base');assert.equal(target.newBranch,true);assert.equal(f.writes.length,0);
});
test('missing identity, unsupported fork, unknown branch ownership and bot actor fail closed',async()=>{
  const missing=fixture();missing.env.WORKER_LOGIN='';await assert.rejects(resolve(missing),/identity/);
  const fork=fixture({fork:true});fork.context.payload.issue.pull_request={};await assert.rejects(resolve(fork),/Unsupported PR/);
  const orphan=fixture({pr:false});await assert.rejects(resolve(orphan),/manual recovery/);
  const bot=fixture();bot.context.payload.comment.user.type='Bot';await assert.rejects(resolve(bot),/Unsupported event/);
  const denied=fixture();denied.github.rest.repos.getCollaboratorPermissionLevel=async()=>({data:{permission:'read'}});await assert.rejects(resolve(denied),/permission/);
});
test('closed issue PR is not recreated from main',async()=>{
  const f=fixture({sha:null,pr:false});f.github.paginate=async()=>[f.targetPR];
  await assert.rejects(resolve(f),/mismatch/);
  f.github.paginate=async(_fn,args)=>args.state==='closed'?[f.targetPR]:[];
  await assert.rejects(resolve(f),/closed/);
});
test('same-repo identity plus branch routing; prefixes alone grant no access',()=>{
  const f=fixture();assert.equal(supportedPR(f.targetPR,'o/r',f.env.WORKER_LOGIN),true);
  assert.equal(supportedPR(f.targetPR,'o/r','other'),false);
  assert.equal(supportedPR({...f.targetPR,head:{...f.targetPR.head,repo:null}},'o/r',f.env.WORKER_LOGIN),false);
  assert.equal(supportedPR({...f.targetPR,draft:true},'o/r',f.env.WORKER_LOGIN),true);
});
test('malformed handoff, hidden control paths, traversal, instructions and modes rejected',()=>{
  for(const value of ['',null,'x'.repeat(220001),'{}','{"patch":""}','{"patch":42}','{"patch":"x","extra":1}'])assert.throws(()=>parsePatch(value));
  for(const value of ['.github/workflows/a.yml','a/../b','a/.env','AGENTS.md','x/AGENTS.override.md','/a','x\\y','x\ny','a//b','a/.codex/config.toml','node_modules/x','target/x'])assert.equal(safePath(value),false,value);
  assert.equal(safePath('crates/example/src/lib.rs'),true);
});
test('publication re-resolves head, validates first and atomically compares expectedHeadOid',async()=>{
  const f=fixture();const target=await resolve(f);f.env.TARGET=JSON.stringify(target);f.env.PATCH='{"patch":"fixture"}';
  await publish({...f,directory:'unused',validate:()=>({additions:[{path:'src/a.rs',contents:'YQ=='}],deletions:[]})});
  assert.equal(f.writes.length,1);assert.equal(f.writes[0].input.expectedHeadOid,'head');assert.equal(f.writes[0].input.branch.branchName,'codex/issue-12');
  f.env.TARGET=JSON.stringify({...target,sha:'old'});f.writes.length=0;
  await assert.rejects(publish({...f,directory:'unused',validate:()=>{throw new Error('must not run');}}),/moved/);assert.equal(f.writes.length,0);
});
test('changed publishing identity or patch-validation failure cannot publish',async()=>{
  const f=fixture();f.env.TARGET=JSON.stringify(await resolve(f));f.env.PATCH='{"patch":"fixture"}';
  await assert.rejects(publish({...f,directory:'unused',validate:()=>{throw new Error('invalid patch');}}),/invalid patch/);assert.equal(f.writes.length,0);
  f.github.graphql=async()=>({viewer:{login:'wrong'}});
  await assert.rejects(publish({...f,directory:'unused',validate:()=>{throw new Error('must not run');}}),/identity/);assert.equal(f.writes.length,0);
});
test('a target moving during validation cannot publish',async()=>{
  const f=fixture();f.env.TARGET=JSON.stringify(await resolve(f));f.env.PATCH='{"patch":"fixture"}';
  await assert.rejects(publish({...f,directory:'unused',validate:()=>{
    f.targetPR.head.sha='moved';return {additions:[],deletions:[]};
  }}),/manual recovery|raced/);
  assert.equal(f.writes.length,0);
});
test('new issue publication creates a missing ref, uses atomic head comparison and reuses a racing matching PR',async()=>{
  const f=fixture({sha:null,pr:false});f.env.TARGET=JSON.stringify(await resolve(f));f.env.PATCH='{"patch":"fixture"}';
  const originalGraphql=f.github.graphql;
  f.github.graphql=async(q,vars)=>{
    const result=await originalGraphql(q,vars);
    if(vars) f.github.paginate=async()=>[f.targetPR];
    return result;
  };
  await publish({...f,directory:'unused',validate:()=>({additions:[{path:'src/a.rs',contents:'YQ=='}],deletions:[]})});
  assert.equal(f.writes.length,2);assert.equal(f.writes[0].ref,'refs/heads/codex/issue-12');assert.equal(f.writes[1].input.expectedHeadOid,'base');
});
test('Git validates patch without running candidate code and exports exact indexed bytes',()=>{
  const temp=fs.mkdtempSync(path.join(os.tmpdir(),'codex-patch-test-'));
  const git=(...args)=>execFileSync('git',args,{cwd:temp,encoding:'utf8'});
  try{
    git('init','-q');git('config','user.name','Test Fixture');git('config','user.email','fixture@example.invalid');git('config','core.autocrlf','false');
    fs.mkdirSync(path.join(temp,'src'));fs.writeFileSync(path.join(temp,'src/a.rs'),'old\n');git('add','.');git('commit','-qm','fixture');
    const patch='diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n';
    assert.throws(()=>validatePatch(temp,patch,'wrong'),/expected exact head/);
    const result=validatePatch(temp,patch);assert.equal(result.additions[0].path,'src/a.rs');assert.equal(Buffer.from(result.additions[0].contents,'base64').toString(),'new\n');
    git('reset','--hard','HEAD');
    assert.throws(()=>validatePatch(temp,patch.replaceAll('src/a.rs','.github/evil.yml')),/Unsafe patch path/);
    assert.throws(()=>validatePatch(temp,'diff --git a/link b/link\nnew file mode 120000\n--- /dev/null\n+++ b/link\n@@ -0,0 +1 @@\n+src/a.rs\n'),/Unsafe staged change/);
  }finally{
    // Test-only directory, bounded under the OS temp directory.
    assert.ok(path.resolve(temp).startsWith(path.resolve(os.tmpdir())+path.sep));fs.rmSync(temp,{recursive:true,force:true});
  }
});

test('live workflow is an inert hold with no blocked action or credential references', () => {
  const active = fs.readFileSync(path.join(__dirname, '../workflows/codex.yml'), 'utf8');
  assert.match(active, /if: \$\{\{ false &&/);
  assert.doesNotMatch(active, /uses:|secrets\.|workflow_dispatch:|pull_request_target:/);
  assert.match(active, /timeout-minutes: 1/);
});
