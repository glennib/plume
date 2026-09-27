-- COPY ... TO with the duckers formats from the shell, with paths relative to the working directory.
.mode csv
COPY (SELECT chart().draw_series(line_series(i, i * i)) FROM range(10) r(i))
TO 'build/test-cli/copy.png' (FORMAT png, WIDTH 320, HEIGHT 200);
COPY (SELECT chart()) TO 'build/test-cli/copy.svg';
SELECT filename, hex(content[1:8]) AS signature, ('0x' || hex(content[17:20]))::INTEGER AS width
FROM read_blob('build/test-cli/copy.png') UNION ALL
SELECT filename, content[1:30], NULL FROM read_text('build/test-cli/copy.svg')
ORDER BY filename;
