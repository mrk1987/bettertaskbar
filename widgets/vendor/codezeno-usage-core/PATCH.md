# CodeZeno usage core

Source: https://github.com/CodeZeno/Claude-Code-Usage-Monitor/tree/a2b2b31ba45dea921a36bb8002309f39c4b11f26
Copyright (C) 2026 Code Zeno Pty Ltd. MIT; full notice in LICENSE.

Only the provider polling, models, credential discovery and supporting modules
are included. BetterTaskbar owns the dashboard, widget windows and themes.
The small app_settings adapter stores non-secret Codex credit baselines under
BetterTaskbar/usage-state. Upstream diagnostic bodies are disabled.
Claude Messages API inference probes and Claude/Codex/Grok CLI inference-based
refresh paths are not used. Expired sessions require sign-in in their owning app.
Upstream source and provider parsing are otherwise retained for maintenance.

Provider/cache tests are retained. Assertions for the upstream expression engine
and upstream HTTPS test harness are excluded because those components are not
part of this library. BetterTaskbar tests its own renderer/binding model.
The Grok shim test waits 30 seconds instead of 5 for the version command,
because PowerShell's first start on a fresh CI runner exceeded 5 seconds.
