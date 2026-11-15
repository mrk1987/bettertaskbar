# Next Departure widget

The next departure from a stop you choose, on two lines:

```
31 · 6 min
Traffic: High
```

The icon shows the transport mode of that departure: bus, tram, metro, train
or boat, so the text gives only the line. A stop served by several modes
changes icon as the next departure changes. The countdown runs live between
refreshes. A delay of two minutes or more is added after the countdown
(`6 min +3`). A departure an hour or more away shows its clock time. Cancelled
departures say so. The pill keeps one width; a longer line, such as one with a
delay, scrolls.

Off by default. Switch it on under **Widgets** in the dashboard.

## Configure

Open the dashboard's **Next Departure** page:

- **Find a stop:** search by name. Choosing a result selects it and saves it
  under **Saved stops**. Switch between saved stops from the widget's
  right-click **Stop** menu.
- **Lines:** line numbers to show, comma separated (`31, 37`). Blank shows
  every line.
- **Direction:** shows only departures whose destination contains this text
  (`Tonsenhagen`). Blank shows every direction.
- **Traffic:** whether to read road situations, and how far from the route
  they count (300 m by default).

In [widgets.json](configuration.md) the settings are under `departures`:

```json
"departures": {
  "stop": { "id": "NSR:StopPlace:58366", "name": "Jernbanetorget, Oslo", "lat": 59.9119, "lon": 10.7504 },
  "favorites": [],
  "lines": ["31"],
  "direction": "",
  "refresh_secs": 60,
  "road_traffic": true,
  "traffic_radius_m": 300
}
```

## The traffic level

**None / Low / Medium / High**, or **Unknown**. It comes from two sources:

- **Road situations** from Statens vegvesen: accidents, closures, roadworks
  and obstructions within the radius of the route. The route is the stops the
  departure on screen visits after yours, up to 20 of them. Accidents and
  obstructions count at least Medium. Closures count High. Works that apply
  only some hours, such as night closures, count Low at most, because the
  public data does not say which hours.
- **Disruption messages** from Entur for the departure and its line, counted
  only while they are in effect. A closure announced for next week does not
  count today.

The level is the worse of the two. It never comes from the bus running late.

For buses and trams, **None** needs road data. If road data is switched off,
cannot be fetched or is more than 20 minutes old, the level is **Unknown**,
unless a disruption message gives a level of its own. For metro, train and
boat, road traffic does not apply. Entur's disruption messages decide the level
there, and no disruption is **None**.

Run `bettertaskbar-widgets.exe --check=departures | more` to see the level and
the situation behind it.

## Data

- Departures, stop search and disruptions:
  [Entur](https://developer.entur.org/) Journey Planner v3 and Geocoder,
  under the [NLOD](https://data.norge.no/nlod/en/2.0) licence. Entur asks
  clients to identify themselves, so requests carry the header
  `ET-Client-Name: bettertaskbar-widgets`. Only the stop id and the query
  text are sent.
- Road situations: Statens vegvesen's DATEX II situations through their open
  WFS service (`ogckart-sn1.atlas.vegvesen.no`, layer
  `datex_3_1:SituationSimple_v2`), under NLOD. Only active situations inside a
  box around the route are requested. The box is about 60 KB for a city route.
  Each route is read at most every 5 minutes.

The authenticated DATEX II feed was not used. It needs an organisation number
and a fixed IP address to register, and it returns the whole country (30 MB or
more) on every request.

## Troubleshooting

- **Choose a stop in settings:** no stop is saved yet.
- **No departures:** nothing matches the line and direction filters in the
  next four hours.
- **Traffic: Unknown** on a bus: check that `ogckart-sn1.atlas.vegvesen.no`
  can be reached, and that **Read road situations** is on.
