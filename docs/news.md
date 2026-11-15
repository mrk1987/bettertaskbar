# News widget

Headlines from news feeds, one at a time, with the source and how old the
story is (`NRK · 12 min`). Double-click opens the headline on screen.

Off by default. Switch it on under **Widgets** in the dashboard.

## Sources

The built-in sources are defaults. Switch any of them off, reorder or remove
them, or add any RSS or Atom feed by name and URL on the dashboard's **News**
page. **Add missing built-in sources** brings back removed defaults, switched
off.

| Source | Feed | On by default |
| --- | --- | --- |
| NRK | `https://www.nrk.no/toppsaker.rss` | yes |
| VG | `https://www.vg.no/rss/feed` | |
| Aftenposten | `https://www.aftenposten.no/rss` | |
| E24 | `https://e24.no/rss2/` | |
| BBC News | `https://feeds.bbci.co.uk/news/rss.xml` | yes |
| The Guardian | `https://www.theguardian.com/world/rss` | |
| Ars Technica | `https://feeds.arstechnica.com/arstechnica/index` | |
| The Verge | `https://www.theverge.com/rss/index.xml` | |
| Hacker News | `https://hnrss.org/frontpage` | |

Each feed answered on 2026-11-14. The AP and Dagbladet feeds that were tried
did not answer, so they are not included. The widget shows only titles and links to each
outlet's own page, which is what these feeds are published for. Most feeds
list the newest stories first, which is not always the outlet's own ranking of
top stories.

## Rotation

- A new headline every 10 seconds (3 to 600).
- Sources take turns, up to six headlines each, so one busy feed cannot crowd
  out the others.
- The same story is shown once, whether two feeds share the link or the title.
- Headlines older than 24 hours are left out, unless nothing newer exists.
- **Pause on hover:** the headline stands still while the pointer is on the
  widget (on by default).
- Any press holds the headline for three seconds, so a double-click always
  opens the story the first click landed on, even with pause on hover off.

## Offline and stale

When a refresh fails, the widget keeps the last headlines and adds
`offline` to the second line. The same happens when the newest reading is more
than three refresh intervals old. A source that fails while others work is
logged and skipped.

## Double-click

The default action, **Open displayed headline**, opens the article on screen in
your browser. Only `http` and `https` links are opened. Change the action in
the dashboard or from the right-click **Double-click action** menu.

## Configure in widgets.json

```json
"news": {
  "feeds": [
    { "name": "NRK", "url": "https://www.nrk.no/toppsaker.rss", "enabled": true }
  ],
  "refresh_secs": 900,
  "rotate_secs": 10,
  "pause_on_hover": true,
  "max_age_hours": 24
}
```

Run `bettertaskbar-widgets.exe --check=news | more` to list what the widget
would rotate through.
