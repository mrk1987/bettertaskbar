# TODO

Widgets should provide compact, at-a-glance information. Avoid turning them
into interactive apps; opening a relevant destination can remain a secondary
action.

## Planned widgets

- [ ] **Now Playing:** show the current track, artist and playback state.
  Keep it informational and compact; a double-click may open the player.
- [ ] **Weather enrichment:** add a small amount of useful information to the
  existing weather widget, keeping its compact size. A short rain-onset or
  daylight hint is a candidate; the exact extra field is not yet selected.
  Avoid adding a separate widget or a full forecast panel for this.
- [ ] **Next Departure:** show upcoming bus/train departures for saved stops,
  with line/direction filtering, countdowns and delays. Include a simple traffic
  indicator for the selected bus route or journey: **None / Low / Medium /
  High**. Keep the widget on two lines to maintain a consistent compact size:
  line 1 `Bus 31 · 6 min`, line 2 `Traffic: High`. Keep road disruptions within
  this widget rather than adding a separate traffic widget or map. Use
  **Unknown** when relevant traffic data is unavailable or stale; do not imply
  clear roads. Select and verify transport and route-relevant traffic sources
  before implementation; do not derive congestion solely from bus lateness.
- [ ] **News / RSS headlines:** rotate headlines from selectable major news
  outlets, with custom RSS/Atom feed URLs. Show the source and publication age.
  Double-click opens the article for the headline currently displayed, rather
  than a fixed homepage. Add this dynamic action to the existing action settings
  and right-click action menu. Keep the headline stable during interaction so
  a double-click cannot open the next rotating story. Include rotation speed,
  pause-on-hover, duplicate handling and a stale/offline indicator. Verify feed
  availability and usage terms before choosing outlet presets; feed order may
  represent latest stories rather than editorially ranked top headlines.

## Suggestions awaiting selection

These are ideas, not approved implementation tasks.

- **Parcel tracking:** next delivery and latest tracking update; open tracking.
- **Electricity price:** current hourly price and today's cheapest period;
  open the day's price chart.
- **World clock:** a second time zone and local date; open additional zones.
- **GitHub activity:** unread notifications, review requests or failed checks;
  open the corresponding item.
- **Quick note:** a pinned reminder or next checklist item.

## Not planned

Andreas declined these on 2026-11-14: network traffic, homelab health, timer,
microphone status and download progress. Audio output switching is also out:
EarTrumpet already covers that need. Sports scores are out too; Andreas is
not interested in sports.
