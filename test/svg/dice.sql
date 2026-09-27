-- Integer buckets counting rows, on a segmented integer axis.
SELECT chart().draw_series(histogram_vertical((i * 7 + i // 3) % 6 + 1, 1).label('rolls')).to_svg()
FROM range(200) r(i);
