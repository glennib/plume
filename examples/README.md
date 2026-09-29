# Example data

`weather.csv` is the daily weather of 2024 in Oslo, Bergen and Tromsø, one row per city and day (1098 rows).
The examples in the [README](../README.md) query it directly, as `FROM 'examples/weather.csv'`,
so they run from the repository root.

| Column | Type | Meaning |
|---|---|---|
| `city` | `VARCHAR` | `Oslo`, `Bergen` or `Tromsø` |
| `day` | `DATE` | the day, in Norwegian time |
| `temp_min` | `DOUBLE` | the day's minimum temperature at 2 m, °C |
| `temp` | `DOUBLE` | the day's mean temperature at 2 m, °C |
| `temp_max` | `DOUBLE` | the day's maximum temperature at 2 m, °C |
| `precipitation` | `DOUBLE` | the day's precipitation, mm |
| `weather` | `VARCHAR` | the day's most severe weather: `clear`, `cloudy`, `drizzle`, `rain` or `snow` |

`weather` groups the day's WMO weather code: 0–1 is `clear`, 2–3 `cloudy`, 51–57 `drizzle`,
61–67 `rain` and 71–77 `snow`.

## Source

The data is ERA5 reanalysis from the
[Open-Meteo historical weather API](https://open-meteo.com/en/docs/historical-weather-api), licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) ([Open-Meteo](https://open-meteo.com/); Hersbach et al., ERA5,
Copernicus Climate Change Service).
Reanalysis is a model of the weather on a grid of about 25 km,
so its values differ somewhat from a station's measurements.

`scripts/fetch_weather.py` downloads it again and writes the file.
