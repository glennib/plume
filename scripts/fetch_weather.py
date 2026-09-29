#!/usr/bin/env python3
"""Fetch the example dataset, examples/weather.csv, from the Open-Meteo historical weather API.

One row per city and day of 2024, for Oslo, Bergen and Tromsø: the daily minimum, mean and
maximum temperature at 2 m (°C), the precipitation sum (mm), and the day's WMO weather code
reduced to one word: clear (0-1), cloudy (2-3), drizzle (51-57), rain (61-67) or snow (71-77).
The data is ERA5 reanalysis, served by Open-Meteo under CC BY 4.0.

Standard library only, so it runs under any Python 3.

Usage: fetch_weather.py [OUTPUT]
"""

import csv
import json
import sys
import urllib.parse
import urllib.request
from pathlib import Path

CITIES = [("Oslo", 59.91, 10.75), ("Bergen", 60.39, 5.32), ("Tromsø", 69.65, 18.96)]
DAILY = [
    "temperature_2m_min",
    "temperature_2m_mean",
    "temperature_2m_max",
    "precipitation_sum",
    "weather_code",
]


def weather(code: int) -> str:
    if code <= 1:
        return "clear"
    if code <= 3:
        return "cloudy"
    if code < 60:
        return "drizzle"
    if code < 70:
        return "rain"
    return "snow"


def fetch(lat: float, lon: float) -> dict:
    query = urllib.parse.urlencode(
        {
            "latitude": lat,
            "longitude": lon,
            "start_date": "2024-01-01",
            "end_date": "2024-12-31",
            "daily": ",".join(DAILY),
            "timezone": "Europe/Oslo",
        }
    )
    url = f"https://archive-api.open-meteo.com/v1/archive?{query}"
    with urllib.request.urlopen(url) as response:
        return json.load(response)["daily"]


def main() -> int:
    output = Path(sys.argv[1] if len(sys.argv) > 1 else "examples/weather.csv")
    with output.open("w", encoding="utf-8", newline="") as f:
        out = csv.writer(f, lineterminator="\n")
        out.writerow(
            ["city", "day", "temp_min", "temp", "temp_max", "precipitation", "weather"]
        )
        for city, lat, lon in CITIES:
            daily = fetch(lat, lon)
            for i, day in enumerate(daily["time"]):
                out.writerow(
                    [
                        city,
                        day,
                        daily["temperature_2m_min"][i],
                        daily["temperature_2m_mean"][i],
                        daily["temperature_2m_max"][i],
                        daily["precipitation_sum"][i],
                        weather(daily["weather_code"][i]),
                    ]
                )
    print(f"{output}: written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
