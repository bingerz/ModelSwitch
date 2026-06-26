export interface ParsedRow {
  name: string;
  group: string | null;
}

export interface CsvResult {
  rows: ParsedRow[];
  errors: string[];
}

const MAX_ROWS = 500;

/**
 * Parse CSV text into rows. Handles basic quoting (double-quote wrapped fields)
 * and escaped quotes inside quoted fields (two consecutive double-quotes).
 *
 * First line is treated as the header row. The `name` column is required;
 * `group` is optional. All other columns are ignored.
 */
export function parseCsv(text: string): CsvResult {
  const lines: string[] = [];
  let current = "";
  let inQuotes = false;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '"') {
      if (inQuotes && text[i + 1] === '"') {
        // Preserve the escaped pair — parseLine will unescape it.
        current += '""';
        i++;
      } else {
        inQuotes = !inQuotes;
        current += ch;
      }
    } else if ((ch === "\n" || ch === "\r") && !inQuotes) {
      if (ch === "\r" && text[i + 1] === "\n") i++;
      if (current.length > 0) lines.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  if (current.length > 0) lines.push(current);

  if (lines.length === 0) {
    return { rows: [], errors: ["CSV file is empty"] };
  }

  const parseLine = (line: string): string[] => {
    const fields: string[] = [];
    let field = "";
    let inQ = false;

    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (ch === '"') {
        if (inQ && line[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          inQ = !inQ;
        }
      } else if (ch === "," && !inQ) {
        fields.push(field);
        field = "";
      } else {
        field += ch;
      }
    }
    fields.push(field);
    return fields.map((f) => f.trim());
  };

  const headers = parseLine(lines[0]).map((h) => h.toLowerCase().trim());
  const nameIdx = headers.indexOf("name");
  const groupIdx = headers.indexOf("group");

  const errors: string[] = [];
  if (nameIdx === -1) {
    errors.push('CSV must contain a "name" column');
    return { rows: [], errors };
  }

  const rows: ParsedRow[] = [];
  for (let i = 1; i < lines.length; i++) {
    const fields = parseLine(lines[i]);
    const name = fields[nameIdx]?.trim() ?? "";
    if (!name) continue;
    const groupVal = groupIdx >= 0 ? (fields[groupIdx]?.trim() ?? "") : "";
    rows.push({
      name,
      group: groupVal || null,
    });
  }

  if (rows.length > MAX_ROWS) {
    errors.push(
      `CSV has ${rows.length} rows, but maximum is ${MAX_ROWS}. Only the first ${MAX_ROWS} will be imported.`,
    );
    return { rows: rows.slice(0, MAX_ROWS), errors };
  }

  return { rows, errors };
}
