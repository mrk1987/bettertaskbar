# TODO

Widgets should provide compact, at-a-glance information. Avoid turning them
into interactive apps; opening a relevant destination can remain a secondary
action.

## Planned widgets

Built 2026-11-14: Now Playing, weather rain onset, Next Departure (with
transport-mode icons) and News. See `docs/now-playing.md`,
`docs/departures.md`, `docs/news.md` and `docs/weather.md`.

- [ ] **Now Playing:** confirm a real track is read on the live machine
  (`--check=now-playing` while something plays). It was only checked with
  nothing playing.
- [ ] **Next Departure:** the open Vegvesen WFS layer gives no hours for
  recurring works, so night closures are capped at Low all day. Revisit if
  the full DATEX feed or a richer layer becomes practical.

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
