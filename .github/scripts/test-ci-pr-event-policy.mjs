import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { Lexer, Parser, Evaluator, data } from '@actions/expressions';

process.chdir(fileURLToPath(new URL('../../', import.meta.url)));

function readWorkflow(path) {
  return JSON.parse(execFileSync('ruby', ['-rjson', '-rpsych', '-e', 'print JSON.generate(Psych.safe_load(File.read(ARGV[0]), aliases: false))', path], { encoding: 'utf8' }));
}
const productionWorkflow = readWorkflow('.github/workflows/ci-pr.yml');
// Before activation, exercise the exact proposed format without changing scheduling.
const workflow = productionWorkflow.jobs.lint.name.startsWith('${{') ? productionWorkflow : readWorkflow('.github/scripts/fixtures/quality-gates-contract/ci-pr.yml');
function convert(value) {
  if (value == null) return new data.Null();
  if (typeof value === 'string') return new data.StringData(value);
  if (typeof value === 'boolean') return new data.BooleanData(value);
  return new data.Dictionary(...Object.entries(value).map(([key, item]) => ({ key, value: convert(item) })));
}
function evaluate(expression, github) {
  const text = expression.startsWith('${{') ? expression.slice(3, -2) : expression;
  const funcs = [{ name: 'always', minArgs: 0, maxArgs: 0, call: () => new data.BooleanData(true) }];
  const tree = new Parser(new Lexer(text).lex().tokens, ['github'], funcs).parse();
  return new Evaluator(tree, convert({ github }), new Map(funcs.map(f => [f.name, f]))).evaluate();
}
const cases = [
  ['opened', 'pull_request', 'opened', undefined, false],
  ['reopened', 'pull_request', 'reopened', undefined, false],
  ['code update', 'pull_request', 'synchronize', undefined, false],
  ['ready', 'pull_request', 'ready_for_review', undefined, false],
  ['title', 'pull_request', 'edited', { title: { from: 'old' } }, true],
  ['body', 'pull_request', 'edited', { body: { from: '' } }, true],
  ['title and body', 'pull_request', 'edited', { title: { from: '' }, body: { from: null } }, true],
  ['base', 'pull_request', 'edited', { base: { ref: { from: 'other' } } }, false],
  ['base and title', 'pull_request', 'edited', { base: {}, title: {} }, false],
  ['unknown edited', 'pull_request', 'edited', {}, false],
  ['missing changes', 'pull_request', 'edited', undefined, false],
  ['merge queue', 'merge_group', 'checks_requested', undefined, false],
];
for (const [label, eventName, action, changes, metadata] of cases) {
  const github = { event_name: eventName, event: { action, ...(changes === undefined ? {} : { changes }) } };
  const canonical = new Set();
  for (const [id, job] of Object.entries(workflow.jobs)) {
    const name = evaluate(job.name, github).coerceString();
    assert.equal(name.startsWith('Metadata only / '), metadata, `${label}: ${id} check identity`);
    assert.equal(evaluate(job.if, github).value, !metadata, `${label}: ${id} execution`);
    canonical.add(name.replace('Metadata only / ', ''));
  }
  assert.equal(canonical.size, 17, `${label}: complete, distinct canonical jobs`);
}
// A failed/running code check cannot be replaced by a skipped metadata check:
// the platform receives disjoint names, regardless of the prior result.
for (const prior of ['failure', 'in_progress', 'cancelled', 'success']) {
  const checks = new Map([['Lint & Format Check', prior]]);
  const github = { event_name: 'pull_request', event: { action: 'edited', changes: { body: { from: '' } } } };
  // GitHub may expose the unevaluated expression for job-level skipped checks.
  for (const name of [evaluate(workflow.jobs.lint.name, github).coerceString(), workflow.jobs.lint.name.slice(3, -2).trim()]) {
    assert.notEqual(name, 'Lint & Format Check');
    checks.set(name, 'skipped');
    assert.equal(checks.get('Lint & Format Check'), prior);
  }
}
console.log(`ci-pr event policy: ${cases.length} event scenarios and check identity isolation passed`);

execFileSync('python3', ['-c', `
import copy,json,runpy,sys
m=runpy.run_path('.github/scripts/check_quality_gates_contract.py')
payload=json.load(sys.stdin)
workflow=payload['workflow']
normalize=m['normalize_ci_metadata_jobs']
normalize(copy.deepcopy(payload['productionWorkflow']))
static=copy.deepcopy(workflow)
normalize(static)
normalize(copy.deepcopy(static))
for job_id in workflow['jobs']:
    for source,target in [(static,workflow),(workflow,static)]:
        changed=copy.deepcopy(target)
        changed['jobs'][job_id]=copy.deepcopy(source['jobs'][job_id])
        try:
            normalize(changed)
        except m['ContractError']:
            continue
        raise AssertionError((job_id,'mixed workflow formats accepted'))
    changed=copy.deepcopy(static)
    changed['jobs'][job_id]['if']='$'+'{{ false }}'
    try:
        normalize(changed)
    except m['ContractError']:
        continue
    raise AssertionError((job_id,'unsafe static guard accepted'))
print('ci-pr rollout: static and wrapped formats accepted; mixed formats and static guard drift rejected')
for job_id in workflow['jobs']:
    for field,value in [('name','Lint & Format Check'),('if','always()')]:
        changed=copy.deepcopy(workflow)
        changed['jobs'][job_id][field]=value
        try:
            normalize(changed)
        except m['ContractError']:
            continue
        raise AssertionError((job_id,field,'unsafe wrapper accepted'))
print('ci-pr metadata wrapper mutation checks passed')
`], { input: JSON.stringify({ workflow, productionWorkflow }), stdio: ['pipe','inherit','inherit'], env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' } });
