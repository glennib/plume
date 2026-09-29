-- The README's opening picture and its grid example, verbatim: four charts of
-- examples/weather.csv in a 2x2 grid (split_evenly) under one title.
SELECT split_evenly([
         (SELECT chart()
                   .configure_mesh().x_labels(4).x_label_formatter('%b').y_label_formatter('{:.0f}').draw()
                   .draw_series(line_series(week, temp, key := city))
                   .configure_series_labels().position('upper_left').background_style('white').draw()
          FROM (SELECT city, date_trunc('week', day)::DATE AS week, avg(temp) AS temp
                FROM 'examples/weather.csv' GROUP BY ALL))
           .titled('Weekly mean temperature (°C)'),
         (SELECT chart().configure_mesh().y_label_formatter('{:.0f}').draw()
                   .draw_series(boxplot_vertical(city, temp).style('teal_600'))
          FROM 'examples/weather.csv')
           .titled('Daily mean temperature (°C)'),
         (SELECT chart().configure_mesh().y_label_formatter('{:.0f}').draw()
                   .draw_series(histogram_vertical(city, mm, order_by := -mm).style('blue_400'))
          FROM (SELECT city, sum(precipitation) AS mm FROM 'examples/weather.csv' GROUP BY city))
           .titled('Precipitation (mm)'),
         (SELECT pie(days, weather, order_by := -days).start_angle(-90).label_style(11).percentages(10)
          FROM (SELECT weather, count(*) AS days FROM 'examples/weather.csv'
                WHERE city = 'Bergen' GROUP BY weather))
           .titled('Days in Bergen'),
       ], 2, 2)
         .root_fill('grey_100')
         .titled('Weather in 2024', 28)
         .to_svg(800, 600);
