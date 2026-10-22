const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {validTarget,reviewText,publish} = require('./review_publication.cjs');
const workflow = fs.readFileSync(path.join(__dirname, '../automation-templates/review-router.yml.disabled'), 'utf8');
const expected = {number:12,head:{sha:'head',repo:{full_name:'o/r'}},base:{sha:'base',repo:{full_name:'o/r'}}};
const pr = {...expected,state:'open',draft:false};
test('all jobs fail closed, forks rejected, generation has no write credentials', () => {
  const jobs = workflow.split(/^  [a-z-]+:\n/gm).slice(1);
  assert.equal(jobs.length, 3);
  for (const job of jobs) {
    assert.match(job, /if: \$\{\{ false &&/);
    assert.match(job, /head.repo.full_name == github.repository/);
    assert.match(job, /persist-credentials: false/);
    assert.match(job, /timeout-minutes:/);
  }
  for (const job of jobs.slice(0, 2)) {
    assert.doesNotMatch(job, /: write|PAT|GH_TOKEN|id-token|pull_request_target/);
    assert.match(job, /permissions: \{contents: read, pull-requests: read}/);
  }
  assert.match(jobs[0], /permission-profile: ':read-only'/);
  assert.match(jobs[0], /safety-strategy: drop-sudo/);
  assert.match(jobs[1], /github_token: \$\{\{ github.token }}/);
  assert.match(jobs[1], /--tools Read,Glob,Grep/);
  assert.doesNotMatch(workflow, /startsWith|yourname|allow-bots|allowed_bots|anthropic_api_key/);
  assert.match(jobs[2], /ref: \$\{\{ github.event.pull_request.base.sha }}/);
  for (const use of workflow.matchAll(/uses: ([^\s]+)/g)) assert.match(use[1], /@[a-f0-9]{40}$/);
});
test('publication checks both exact heads, repo identity and open state', () => {
  assert.equal(validTarget(pr,'o/r',expected), true);
  for (const changed of [{...pr,head:{...pr.head,sha:'new'}},{...pr,base:{...pr.base,sha:'new'}},{...pr,head:{...pr.head,repo:{full_name:'fork/r'}}},{...pr,state:'closed'},{...pr,draft:true}]) assert.equal(validTarget(changed,'o/r',expected),false);
});
test('bounded review text neutralizes bot mentions; malformed output fails', () => {
  assert.equal(reviewText('@codex fix'), '@\u200bcodex fix');
  for (const value of ['',null,'x'.repeat(20001),'bad\0']) assert.throws(()=>reviewText(value));
});
test('a raced head never reaches the publication API; successful review is COMMENT on exact commit', async () => {
  let posts = [];
  const github={rest:{pulls:{get:async()=>({data:{...pr,head:{...pr.head,sha:'moved'}}}),createReview:async x=>posts.push(x)}}};
  const args={github,context:{repo:{owner:'o',repo:'r'},payload:{pull_request:expected}},env:{CODEX_REVIEW:'one',CLAUDE_REVIEW:'{"review":"two"}'}};
  await assert.rejects(publish(args), /moved/);
  assert.equal(posts.length,0);
  github.rest.pulls.get=async()=>({data:pr});
  await publish(args);
  assert.equal(posts[0].commit_id,'head');
  assert.equal(posts[0].event,'COMMENT');
});

test('live workflow is an inert hold with no blocked action or credential references', () => {
  const active = fs.readFileSync(path.join(__dirname, '../workflows/review-router.yml'), 'utf8');
  assert.match(active, /if: \$\{\{ false &&/);
  assert.doesNotMatch(active, /uses:|secrets\.|pull_request(?:_target)?:/);
  assert.match(active, /^on:\n  workflow_dispatch:\npermissions:/m);
  assert.match(workflow, /^on:\n  pull_request: \{types: \[opened, synchronize, ready_for_review\]\}/m);
  assert.match(active, /timeout-minutes: 1/);
});
