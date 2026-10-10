"""CVM tenant gateway contract, shared by the isolated entry and read-only tools."""
import hmac
import re
import time
import urllib.parse

TENANT = "cvm"
CASE_PREDICATES = {
    "normal": "span.cvm.normal_candidate = true",
    "slow": "span.cvm.slow = true",
    "wait": "span.cvm.high_wait = true",
    "retry": "span.cvm.retry = true",
    "error": '(span.cvm.outcome = "error" || span.cvm.outcome = "cancelled" || span.cvm.status_class = "4xx" || span.cvm.status_class = "5xx")',
}

def case_query(category, environment=None, instance=None, endpoint=None):
    predicate = CASE_PREDICATES[category]
    filters = ['resource.service.name = "codex-vibe-monitor"', 'span.cvm.record = "response"', predicate]
    for key, value in [("resource.deployment.environment.name", environment), ("resource.service.instance.id", instance), ("span.cvm.endpoint", endpoint)]:
        if value and value != "all":
            if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,64}", value):
                raise ValueError("invalid trace filter")
            filters.append(f'{key} = "{value}"')
    return "{ " + " && ".join(filters) + " }"

def authorized(header, expected):
    return bool(expected and hmac.compare_digest((header or "").encode(), ("Bearer " + expected).encode()))

def query_route(target, now=None, fixed_cases=True):
    """Return a private Tempo GET path; reject management and unbounded searches."""
    parsed = urllib.parse.urlsplit(target)
    if parsed.scheme or parsed.netloc or parsed.fragment:
        raise ValueError("invalid trace path")
    if re.fullmatch(r"/api/v2/traces/[a-f0-9]{32}", parsed.path):
        if parsed.query:
            query = urllib.parse.parse_qs(parsed.query, strict_parsing=True)
            if fixed_cases or set(query) != {"start", "end"} or any(len(v) != 1 for v in query.values()):
                raise ValueError("trace lookup parameters are not allowed")
            start, end = (int(query[name][0]) for name in ["start", "end"])
            clock = time.time() if now is None else now
            if not 0 < end - start <= 86400 or start < clock - 86400 - 60 or end > clock + 60:
                raise ValueError("trace lookup exceeds retention bounds")
        return target
    if parsed.path != "/api/search":
        raise ValueError("trace route is not allowed")
    query = urllib.parse.parse_qs(parsed.query, strict_parsing=True)
    if set(query) != {"q", "start", "end", "limit"} or any(len(v) != 1 for v in query.values()):
        raise ValueError("invalid trace search parameters")
    start, end, limit = (int(query[name][0]) for name in ["start", "end", "limit"])
    clock = time.time() if now is None else now
    if not 0 < end - start <= 86400 or start < clock - 86400 - 60 or end > clock + 60 or not 1 <= limit <= 3:
        raise ValueError("trace search exceeds retention or case bounds")
    expression = query["q"][0]
    if len(expression) > 2048 or (fixed_cases and not any(re.fullmatch(re.escape(case_query(category).removesuffix(" }")) + r'(?: && (?:resource\.deployment\.environment\.name|resource\.service\.instance\.id|span\.cvm\.endpoint) = "[A-Za-z0-9_.:-]{1,64}"){0,3} \}', expression) for category in CASE_PREDICATES)):
        raise ValueError("only fixed CVM case queries are allowed")
    return target
