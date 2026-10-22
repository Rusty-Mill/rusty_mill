const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const text = fs.readFileSync(path.join(__dirname, '../workflows/claude.yml'), 'utf8');
test('worker is disabled, human mention gated, bounded and subscription-authenticated', () => {
  assert.match(text, /if: \$\{\{ false &&/);
  assert.match(text, /comment\.user\.type == 'User'/);
  assert.match(text, /contains\(github.event.comment.body, '@claude'\)/);
  assert.match(text, /claude_code_oauth_token: \$\{\{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}/);
  assert.doesNotMatch(text, /anthropic_api_key:|id-token:|yourname|allowed_bots:|pull_request_target:|prompt:/);
  assert.match(text, /github_token: \$\{\{ github.token }}/);
  assert.match(text, /timeout-minutes: 20/);
  assert.match(text, /queue: max/);
  assert.match(text, /cancel-in-progress: false/);
  assert.match(text, /persist-credentials: false/);
  for (const use of text.matchAll(/uses: ([^\s]+)/g)) assert.match(use[1], /@[a-f0-9]{40}$/);
});
test('current PR identity and checkout use API-resolved exact head, never a prefix alone', async () => {
  const script = text.split('          script: |\n')[1].split('      - if:')[0].split('\n').map(x => x.slice(12)).join('\n');
  const fn = new (Object.getPrototypeOf(async function(){}).constructor)('github','context','core',script);
  for (const [repo, ref, state, accepted] of [['Rusty-Mill/rusty_mill','claude/a','open',true], ['outsider/fork','claude/a','open',false], ['Rusty-Mill/rusty_mill','codex/a','open',false], ['Rusty-Mill/rusty_mill','claude/a','closed',false]]) {
    const outputs = {};
    await fn({rest:{pulls:{get:async()=>({data:{state,head:{repo:{full_name:repo},ref,sha:'exact'}}})}}}, {repo:{owner:'Rusty-Mill',repo:'rusty_mill'},payload:{issue:{number:1,pull_request:{}}}}, {notice:()=>{},setOutput:(k,v)=>outputs[k]=v});
    assert.equal(outputs.sha, accepted ? 'exact' : undefined);
  }
});
