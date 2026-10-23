// Loaded only from the trusted base SHA on a fresh publisher runner.
'use strict';
function validTarget(pr, repository, expected) {
  return pr.state === 'open' && !pr.draft &&
    pr.head.repo?.full_name === repository && pr.base.repo?.full_name === repository &&
    pr.head.sha === expected.head.sha && pr.base.sha === expected.base.sha;
}
function reviewText(value) {
  if (typeof value !== 'string' || !value.trim() || value.length > 20000 || /[\x00-\x08\x0b\x0c\x0e-\x1f]/.test(value)) {
    throw new Error('Invalid or oversized review output');
  }
  // Outputs are data, never scripts. Neutralize mentions, including @codex/@claude.
  return value.replace(/@/g, '@\u200b');
}
async function publish({github, context, env = process.env}) {
  const expected = context.payload.pull_request;
  const repository = `${context.repo.owner}/${context.repo.repo}`;
  const parsed = JSON.parse(env.CLAUDE_REVIEW);
  if (!parsed || Object.keys(parsed).join() !== 'review') throw new Error('Invalid Claude schema');
  const body = `Codex findings\n\n${reviewText(env.CODEX_REVIEW)}\n\nClaude findings\n\n${reviewText(parsed.review)}`;
  const {data: current} = await github.rest.pulls.get({...context.repo, pull_number: expected.number});
  if (!validTarget(current, repository, expected)) throw new Error('PR moved or unsupported target; regenerate');
  // commit_id anchors the review even if a push races the API call. Never approve.
  await github.rest.pulls.createReview({...context.repo, pull_number: expected.number,
    commit_id: expected.head.sha, event: 'COMMENT', body});
}
module.exports = {validTarget, reviewText, publish};
