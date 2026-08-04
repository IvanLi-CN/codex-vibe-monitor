# Agent Issue Tracker

- Tracker: GitHub Issues
- Repository: `IvanLi-CN/codex-vibe-monitor`
- Issue URL: `https://github.com/IvanLi-CN/codex-vibe-monitor/issues`
- Agent-ready label: `ready-for-agent`
- Delivery model: issue -> managed Codex task worktree -> child pull request
- Default child PR base for initiatives: the initiative integration branch named in the approved plan

An issue may carry `ready-for-agent` only when its body contains the approved-plan envelope, concrete integration and final bases, dependencies, wave, risk, PR role, and stop condition. Removing the label suspends new agent claims without changing already-open pull requests.
