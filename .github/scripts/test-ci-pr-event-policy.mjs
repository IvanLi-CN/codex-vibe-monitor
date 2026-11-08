import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { Lexer, Parser, Evaluator, data } from "@actions/expressions";
import { truthy } from "@actions/expressions/result";

process.chdir(fileURLToPath(new URL("../../", import.meta.url)));

function readWorkflow(path) {
  return JSON.parse(
    execFileSync(
      "ruby",
      [
        "-rjson",
        "-rpsych",
        "-e",
        "print JSON.generate(Psych.safe_load(File.read(ARGV[0]), aliases: false))",
        path,
      ],
      { encoding: "utf8" },
    ),
  );
}
const productionWorkflow = readWorkflow(".github/workflows/ci-pr.yml");
// Before activation, exercise the exact proposed format without changing scheduling.
const workflow = productionWorkflow.jobs.lint.if?.includes(
  "github.event.changes.title",
)
  ? productionWorkflow
  : readWorkflow(".github/scripts/fixtures/quality-gates-contract/ci-pr.yml");
function convert(value) {
  if (value == null) return new data.Null();
  if (typeof value === "string") return new data.StringData(value);
  if (typeof value === "boolean") return new data.BooleanData(value);
  return new data.Dictionary(
    ...Object.entries(value).map(([key, item]) => ({
      key,
      value: convert(item),
    })),
  );
}
function evaluate(expression, github) {
  const text = expression.startsWith("${{")
    ? expression.slice(3, -2)
    : expression;
  const funcs = [
    {
      name: "always",
      minArgs: 0,
      maxArgs: 0,
      call: () => new data.BooleanData(true),
    },
  ];
  const tree = new Parser(
    new Lexer(text).lex().tokens,
    ["github"],
    funcs,
  ).parse();
  return new Evaluator(
    tree,
    convert({ github }),
    new Map(funcs.map((f) => [f.name, f])),
  ).evaluate();
}
const cases = [
  ["opened", "pull_request", "opened", undefined, false],
  ["reopened", "pull_request", "reopened", undefined, false],
  ["code update", "pull_request", "synchronize", undefined, false],
  ["ready", "pull_request", "ready_for_review", undefined, false],
  ["title", "pull_request", "edited", { title: { from: "old" } }, true],
  ["body", "pull_request", "edited", { body: { from: "" } }, true],
  [
    "title and body",
    "pull_request",
    "edited",
    { title: { from: "" }, body: { from: null } },
    true,
  ],
  [
    "base",
    "pull_request",
    "edited",
    { base: { ref: { from: "other" } } },
    false,
  ],
  ["base and title", "pull_request", "edited", { base: {}, title: {} }, false],
  ["unknown edited", "pull_request", "edited", {}, false],
  ["missing changes", "pull_request", "edited", undefined, false],
  ["merge queue", "merge_group", "checks_requested", undefined, false],
];
for (const [label, eventName, action, changes, metadata] of cases) {
  const github = {
    event_name: eventName,
    event: { action, ...(changes === undefined ? {} : { changes }) },
  };
  const canonical = new Set();
  for (const [id, job] of Object.entries(workflow.jobs)) {
    assert.equal(typeof job.name, "string");
    assert.equal(
      job.name.includes("${{"),
      false,
      `${label}: ${id} canonical name`,
    );
    const scheduled = id === "repository-tooling-checks" || !metadata;
    assert.equal(
      job.if ? evaluate(job.if, github).value : true,
      scheduled,
      `${label}: ${id} execution`,
    );
    canonical.add(job.name);
  }
  assert.equal(
    canonical.size,
    17,
    `${label}: complete, distinct canonical jobs`,
  );
  const tooling = workflow.jobs["repository-tooling-checks"];
  for (const step of tooling.steps) {
    const runs =
      step.if && !step.if.includes("steps.")
        ? truthy(evaluate(step.if, github))
        : true;
    if (step.name === "Validate complete CI evidence")
      assert.equal(runs, metadata);
    else if (
      ![
        "Checkout",
        "Resolve trusted CI event evidence",
        "Upload CI event provenance",
      ].includes(step.name)
    ) {
      // The two existing trusted-source conditions additionally depend on outputs.
      if (!step.if.includes("steps."))
        assert.equal(runs, !metadata, `${label}: ${step.name}`);
      else assert.ok(step.if.includes("!("));
    }
  }
}
console.log(
  `ci-pr event policy: ${cases.length} event scenarios and one-runner scheduling passed`,
);

execFileSync(
  "python3",
  [
    "-c",
    `
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
legacy=copy.deepcopy(static)
for job_id, job in legacy['jobs'].items():
    job['name']=m['ci_metadata_job_name'](job['name'])
    job['if']=m['ci_metadata_job_if'](job_id)
normalize(legacy)
assert legacy == static
print('ci-pr rollout: static, legacy and evidence formats accepted; mixed formats rejected')
for job_id in workflow['jobs']:
    changed=copy.deepcopy(workflow)
    changed['jobs'][job_id]['if']='always()'
    try:
        normalize(changed)
    except m['ContractError']:
        continue
    raise AssertionError((job_id,'unsafe guard accepted'))
tooling=workflow['jobs']['repository-tooling-checks']
for index in range(1,len(tooling['steps'])):
    changed=copy.deepcopy(workflow)
    changed['jobs']['repository-tooling-checks']['steps'][index]['if']='always()'
    try:
        normalize(changed)
    except m['ContractError']:
        continue
    raise AssertionError((index,'unsafe metadata step accepted'))
for field,value in [('permissions',{'contents':'read','actions':'write'}),('timeout-minutes',15)]:
    changed=copy.deepcopy(workflow)
    changed['jobs']['repository-tooling-checks'][field]=value
    try:
        normalize(changed)
    except m['ContractError']:
        continue
    raise AssertionError((field,'unsafe evidence configuration accepted'))
# Startup injection must be rejected before trusting the base reader.
for scope in ('workflow-env','job-env','defaults','checkout-env','container','runner'):
    for key,value in [('PYTHONPATH','.'),('PATH','.'),('BASH_ENV','candidate.sh'),('LD_PRELOAD','candidate.so')]:
        changed=copy.deepcopy(workflow)
        job=changed['jobs']['repository-tooling-checks']
        if scope == 'workflow-env': changed['env'][key]=value
        elif scope == 'job-env': job['env']={key:value}
        elif scope == 'defaults': changed['defaults']={'run':{'shell':'candidate {0}'}}
        elif scope == 'checkout-env': job['steps'][0]['env']={key:value}
        elif scope == 'container': job['container']='candidate:latest'
        else: job['runs-on']='self-hosted'
        try:
            normalize(changed)
        except m['ContractError']:
            continue
        raise AssertionError((scope,key,'candidate startup injection accepted'))
for index in (1,3):
    changed=copy.deepcopy(workflow)
    step=changed['jobs']['repository-tooling-checks']['steps'][index]
    step['run']=step['run'].replace('/usr/bin/python3 -I','python3')
    try:
        normalize(changed)
    except m['ContractError']:
        continue
    raise AssertionError((index,'non-isolated interpreter accepted'))
print('ci-pr metadata evidence guards, trust and startup isolation mutation checks passed')
`,
  ],
  {
    input: JSON.stringify({ workflow, productionWorkflow }),
    stdio: ["pipe", "inherit", "inherit"],
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  },
);

const proofNames = JSON.parse(
  execFileSync(
    "python3",
    [
      "-c",
      "import json,runpy;print(json.dumps(sorted(runpy.run_path('.github/scripts/ci_pr_metadata_evidence.py')['JOB_NAMES'])))",
    ],
    { env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" } },
  ),
);
assert.deepEqual(
  proofNames,
  Object.values(workflow.jobs)
    .map((job) => job.name)
    .sort(),
);
