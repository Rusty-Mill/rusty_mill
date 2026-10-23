// Trusted default-branch code only. Never load this module from an agent checkout.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
function repository(context) { return `${context.repo.owner}/${context.repo.repo}`; }
function supportedPR(pr, repo, login) {
  return pr.state === 'open' && pr.head.repo?.full_name === repo &&
    pr.base.repo?.full_name === repo && pr.head.ref.startsWith('codex/') && pr.user.login === login;
}
async function findRef(github, context, branch) {
  try { return (await github.rest.git.getRef({...context.repo,ref:`heads/${branch}`})).data.object.sha; }
  catch (error) { if (error.status === 404) return null; throw error; }
}
async function resolve({github, context, env = process.env}) {
  const login = env.WORKER_LOGIN;
  if (!login || !/^[a-z\d][a-z\d-]*(\[bot\])?$/i.test(login)) throw new Error('Verified worker identity is not configured');
  const {data: account} = await github.rest.users.getByUsername({username:login});
  if (account.login !== login || !account.id) throw new Error('Worker identity cannot be verified');
  const event = context.payload;
  if (context.eventName !== 'issue_comment' || event.action !== 'created' ||
      event.comment.user.type !== 'User' || !event.comment.body.includes('@codex') ||
      !['OWNER','MEMBER','COLLABORATOR'].includes(event.comment.author_association)) throw new Error('Unsupported event or actor');
  // Association is only a first filter; refresh current collaborator permission.
  const {data: access} = await github.rest.repos.getCollaboratorPermissionLevel({...context.repo,username:event.comment.user.login});
  if (!['admin','maintain','write'].includes(access.permission)) throw new Error('Actor no longer has write permission');
  const repo = repository(context);
  const number = event.issue.number;
  if (!Number.isSafeInteger(number) || number < 1) throw new Error('Invalid issue number');
  if (event.issue.pull_request) {
    const {data: pr} = await github.rest.pulls.get({...context.repo,pull_number:number});
    if (!supportedPR(pr,repo,login)) throw new Error('Unsupported PR identity, repository or branch');
    return {branch:pr.head.ref,sha:pr.head.sha,pr:number,issue:null,newBranch:false,base:pr.base.ref,baseSha:pr.base.sha,login};
  }
  const {data: issue} = await github.rest.issues.get({...context.repo,issue_number:number});
  if (issue.state !== 'open' || issue.pull_request) throw new Error('Issue is not open');
  const branch = `codex/issue-${number}`;
  const sha = await findRef(github,context,branch);
  const prs = await github.paginate(github.rest.pulls.list,{...context.repo,state:'open',head:`${context.repo.owner}:${branch}`,per_page:100});
  if (sha) {
    // Resume the existing exact branch/PR; no checkout -b from main, reset or force.
    if (prs.length !== 1 || !supportedPR(prs[0],repo,login) || prs[0].head.sha !== sha) throw new Error('Existing branch needs one verified open worker PR; manual recovery required');
    const pr=prs[0];
    return {branch,sha,pr:pr.number,issue:number,newBranch:false,base:pr.base.ref,baseSha:pr.base.sha,login};
  }
  if (prs.length) throw new Error('PR/ref mismatch');
  const history = await github.paginate(github.rest.pulls.list,{...context.repo,state:'closed',head:`${context.repo.owner}:${branch}`,per_page:100});
  if (history.length) throw new Error('Prior issue PR is closed; explicit recovery decision required');
  const {data: metadata} = await github.rest.repos.get(context.repo);
  const base = metadata.default_branch;
  const baseSha = await findRef(github,context,base);
  if (!baseSha) throw new Error('Default branch is missing');
  return {branch,sha:baseSha,pr:null,issue:number,newBranch:true,base,baseSha,login};
}
function sameTarget(expected, current) {
  return ['branch','sha','pr','issue','newBranch','base','baseSha','login'].every(k=>expected[k] === current[k]);
}
function safePath(value) {
  return typeof value === 'string' && value.length <= 240 &&
    /^[A-Za-z0-9_][A-Za-z0-9_./-]*$/.test(value) &&
    value.split('/').every(part=>part && !part.startsWith('.') && !/^AGENTS(?:\.override)?\.md$/i.test(part)) &&
    !/(^|\/)(?:node_modules|target|vendor)(\/|$)/i.test(value);
}
function parsePatch(value) {
  if (typeof value !== 'string' || value.length > 220000) throw new Error('Oversized patch handoff');
  const parsed = JSON.parse(value);
  if (!parsed || Object.keys(parsed).join() !== 'patch' || typeof parsed.patch !== 'string' ||
      !parsed.patch.trim() || parsed.patch.length > 200000 || parsed.patch.includes('\0')) throw new Error('Invalid patch schema');
  return parsed.patch;
}
function validatePatch(directory, patch, expectedSha) {
  const cwd = path.resolve(directory);
  // Command arguments are fixed; all patch text is stdin, never shell source.
  const options={cwd,encoding:'utf8',maxBuffer:2000000,env:{...process.env,GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:process.platform === 'win32' ? 'NUL' : '/dev/null'}};
  const git = (...args) => execFileSync('git',['-c','core.hooksPath=/dev/null','-c','core.fsmonitor=false','-c','diff.external=',...args],options);
  if (expectedSha && git('rev-parse','HEAD').trim() !== expectedSha) throw new Error('Publisher checkout is not the expected exact head');
  if (git('status','--porcelain').trim()) throw new Error('Publisher checkout is not clean');
  // A preliminary header check blocks absolute/traversal/control paths before apply.
  const headers = [...patch.matchAll(/^diff --git a\/(\S+) b\/(\S+)$/gm)];
  if (!headers.length || headers.some(m=>m[1] !== m[2] || !safePath(m[1]))) throw new Error('Unsafe patch path');
  if (/^(?:GIT binary patch|Binary files |(?:old|new) mode |rename (?:from|to) |copy (?:from|to) )/m.test(patch)) throw new Error('Unsupported patch operation');
  for (const flags of [['--check','--index'],['--index']]) execFileSync('git',
    ['-c','core.hooksPath=/dev/null','-c','core.fsmonitor=false','apply',...flags,'--whitespace=error','-'],{...options,input:patch});
  const raw=git('diff','--cached','--raw','-z','--no-renames').split('\0');
  const additions=[],deletions=[];
  for (let i=0;i<raw.length-1;i+=2) {
    const fields=raw[i].split(' '), filename=raw[i+1];
    const oldMode=fields[0].slice(1),newMode=fields[1],status=fields[4];
    if (!safePath(filename) || !['000000','100644'].includes(oldMode) || !['000000','100644'].includes(newMode) || !['A','M','D'].includes(status)) throw new Error('Unsafe staged change');
    if (status === 'D') deletions.push({path:filename});
    else {
      const full=path.join(cwd,filename);
      if (!fs.lstatSync(full).isFile() || fs.lstatSync(full).isSymbolicLink()) throw new Error('Not a regular file');
      const contents=execFileSync('git',['-c','core.hooksPath=/dev/null','show',`:${filename}`],{...options,encoding:'buffer'});
      additions.push({path:filename,contents:contents.toString('base64')});
    }
  }
  if (!additions.length && !deletions.length) throw new Error('Empty patch');
  if (additions.length+deletions.length > 30) throw new Error('Patch exceeds 30-file publication limit');
  return {additions,deletions};
}
async function publish({github, context, directory, env = process.env, validate = validatePatch}) {
  const expected = JSON.parse(env.TARGET);
  const patch = parsePatch(env.PATCH);
  const current = await resolve({github,context,env});
  if (!sameTarget(expected,current)) throw new Error('Target moved; regenerate from the new exact head');
  // Verify the actual ephemeral publishing principal, not a fabricated account.
  const viewer=await github.graphql('{ viewer { login } }');
  if (viewer.viewer.login !== expected.login) throw new Error('Publishing token identity differs from configured worker');
  const changes = validate(directory,patch,expected.sha);
  // Recheck after potentially slow validation, before any branch creation/write.
  if (!sameTarget(expected,await resolve({github,context,env}))) throw new Error('Target raced validation');
  if (expected.newBranch) {
    // createRef rejects a racing pre-existing branch; never update or force it.
    await github.rest.git.createRef({...context.repo,ref:`refs/heads/${expected.branch}`,sha:expected.sha});
  }
  // expectedHeadOid is an atomic compare-and-swap: it cannot overwrite a racing push.
  const result=await github.graphql(`mutation($input: CreateCommitOnBranchInput!) {
    createCommitOnBranch(input:$input) { commit { oid } }
  }`, {input:{branch:{repositoryNameWithOwner:repository(context),branchName:expected.branch},
    expectedHeadOid:expected.sha,message:{headline:`Codex correction for #${expected.issue || expected.pr}`},fileChanges:changes}});
  if (!result.createCommitOnBranch?.commit?.oid) throw new Error('Commit outcome not confirmed; inspect before retry');
  if (!expected.pr) {
    // Re-query so a concurrently created PR is reused instead of duplicated.
    const prs=await github.paginate(github.rest.pulls.list,{...context.repo,state:'open',head:`${context.repo.owner}:${expected.branch}`,per_page:100});
    if (prs.length > 1) throw new Error('Ambiguous PR publication');
    if (prs.length === 1) {
      if (!supportedPR(prs[0],repository(context),expected.login)) throw new Error('Unexpected existing PR identity');
      return;
    }
    await github.rest.pulls.create({...context.repo,head:expected.branch,base:expected.base,
      title:`Codex implementation for #${expected.issue}`,body:`Closes #${expected.issue}\n\nHuman-requested patch. Provider validation was not executed by the publication job.`,draft:true});
  }
}
module.exports={resolve,supportedPR,sameTarget,safePath,parsePatch,validatePatch,publish};
