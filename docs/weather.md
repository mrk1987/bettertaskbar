# Weather widget

Yr's symbol for the weather right now, the next 24 hours' high and low temperature, and
the precipitation expected over the next 24 hours, for a place you choose.
Temperatures above zero are red, zero and below blue.

## Configure

No account needed: it uses the public API of MET Norway, the same data as
[yr.no](https://www.yr.no). Settings are under `weather` in
[widgets.json](configuration.md):

```json
"weather": {
  "place": { "name": "Oslo", "lat": 59.9139, "lon": 10.7522 },
  "favorites": [],
  "refresh_secs": 1800
}
```

- `place`: the location shown. Easiest to set from the right-click menu.
- `favorites`: places offered in the right-click menu for a quick switch.
- `refresh_secs`: how often to fetch a new forecast (default 30 minutes).

## Use

- **Right-click:** the current place, your favorites and **Search place…**,
  which opens the dashboard’s Weather page. Search uses Open-Meteo / GeoNames. Picking a result selects it and adds it
  to your favorites. Choose **Save changes** to apply it.
- **Double-click:** opens the forecast for the place on yr.no.
- **Drag:** see [Moving them](../README.md#moving-them).

Place searches send only the name you enter to
[Open-Meteo](https://open-meteo.com/en/docs/geocoding-api). Its free hosted API
is for non-commercial use; commercial deployments need an appropriate service
arrangement. You can always enter coordinates directly in `widgets.json`.
Forecasts still come from MET Norway.

## Data

- Forecast: `api.met.no/weatherapi/locationforecast/2.0/compact`. High and low
  are the maximum and minimum over the next 24 hours; precipitation is the sum
  over the same hours.
- The symbol is Yr's own icon for the forecast's symbol code, including its day,
  night and polar twilight versions, taken from Yr's set for dark backgrounds.

Weather data from MET Norway, licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

## Troubleshooting

- The widget shows `…` until the first forecast arrives. Check that
  `api.met.no` and `www.yr.no` can be reached.
- If an icon cannot be loaded, the temperatures still show.
