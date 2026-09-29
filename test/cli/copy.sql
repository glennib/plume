-- COPY ... TO with the plume formats from the shell, with paths relative to the working directory.
.mode csv
COPY (SELECT chart().draw_series(line_series(i, i * i)) FROM range(10) r(i))
TO 'build/test-cli/copy.png' (FORMAT png, WIDTH 320, HEIGHT 200);
COPY (SELECT chart()) TO 'build/test-cli/copy.svg';
-- read_blob and read_text may name the files with the OS separator (backslashes on Windows).
SELECT replace(filename, '\', '/') AS filename, hex(content[1:8]) AS signature, ('0x' || hex(content[17:20]))::INTEGER AS width
FROM read_blob('build/test-cli/copy.png') UNION ALL
SELECT replace(filename, '\', '/'), content[1:30], NULL FROM read_text('build/test-cli/copy.svg')
ORDER BY filename;
