# TODO

## Planned widgets

- [ ] **Now Playing:** show the current track, artist and playback state.
  Provide playback controls and a way to open the player. Decide how these
  interact with configurable double-click actions before implementation.
- [ ] **Next Departure:** show upcoming bus/train departures for saved stops,
  with line/direction filtering, countdowns and delays. Select and verify the
  transport data source before implementation.
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
- **Sports scores:** live score or next fixture for selected teams; open details.
- **Quick note:** a pinned reminder or short checklist, editable from the widget.

## Not planned

Andreas declined these on 2026-11-14: network traffic, homelab health, timer,
microphone status and download progress. Audio output switching is also out:
EarTrumpet already covers that need.
