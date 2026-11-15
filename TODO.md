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

- [ ] **Norwegian translation (bokmål):** the app is now aimed mostly at
  Norwegians (Yr, Entur, Statens vegvesen, NRK and other Norwegian feeds), so
  offer Norwegian alongside English. Covers the widget text (`Rain 40m`,
  `Dry 6h`, `Traffic: Low`, `No unread mail`, countdowns and ages), the
  right-click and tray menus, and the dashboard. Follow Windows' display
  language by default, with a choice in the dashboard. Keep the compact
  pills' widths: measure them from the longest text in the chosen language,
  not from the English strings. The README and docs can stay in English.

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
