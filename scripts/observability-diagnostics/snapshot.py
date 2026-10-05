"""Run in the synthetic client; return only allowlisted numeric metric sums."""
import sys
sys.path.insert(0, "/work")
import client
from evidence import metric_summary
import json


def main():
    token = (client.ROOT / "metrics-token").read_text().strip()
    status, body = client.request("http://app:9091", "/metrics", token=token)
    if status != 200:
        return {"status": "unknown", "httpStatus": status}
    return metric_summary(body.decode())


if __name__ == "__main__":
    try:
        print(json.dumps(main()))
    except Exception as error:
        print(json.dumps({"status": "unknown", "reason": type(error).__name__}))
