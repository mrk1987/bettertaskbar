# TODO

Widgets should provide compact, at-a-glance information. Avoid turning them
into interactive apps; opening a relevant destination can remain a secondary
action.

## Planned widgets

Built 2026-11-14: Now Playing, weather rain onset, Next Departure (with
transport-mode icons) and News. See `docs/now-playing.md`,
`docs/departures.md`, `docs/news.md` and `docs/weather.md`.

- [ ] **Next Departure:** the open Vegvesen WFS layer gives no hours for
  recurring works, so night closures are capped at Low all day. Revisit if
  the full DATEX feed or a richer layer becomes practical.

## Planned

- [ ] **Norwegian error details:** server and network errors (IMAP,
  CalDAV, feeds, Entur, Vegvesen) and the `--check` output are still English
  in Norwegian mode. The usage providers' own window labels (`5h`, `7d`,
  `30d`) are shown as the providers send them.

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
