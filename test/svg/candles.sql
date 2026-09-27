-- candle_stick(x, open, high, low, close) on a date axis: filled bodies, custom gain and loss colours.
SELECT chart()
         .caption('candle_stick', 20)
         .configure_mesh().x_label_formatter('%b %d').draw()
         .draw_series(candle_stick(day, open, high, low, close)
                        .gain_style('green_600').loss_style('red_600').width(9).filled().label('price'))
         .configure_series_labels().position('upper_right').border_style('black').draw()
         .to_svg(800, 480)
FROM (SELECT day, open, close, greatest(open, close) + 1 + i % 3 AS high, least(open, close) - 1 - i % 2 AS low
      FROM (SELECT DATE '2024-01-01' + i::INTEGER AS day, i, round(100 + 10 * sin(i / 4)) AS open,
                   round(round(100 + 10 * sin(i / 4)) + 3 * cos(i * 1.7)) AS close
            FROM range(30) r(i)));
