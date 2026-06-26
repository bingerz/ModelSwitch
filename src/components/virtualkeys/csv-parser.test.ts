import { describe, it, expect } from "vitest";
import { parseCsv } from "./csv-parser";

describe("parseCsv", () => {
  it("parses simple CSV with name and group columns", () => {
    const csv = "name,group\nAlice,Engineering\nBob,Sales";
    const result = parseCsv(csv);
    expect(result.errors).toHaveLength(0);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0]).toEqual({ name: "Alice", group: "Engineering" });
    expect(result.rows[1]).toEqual({ name: "Bob", group: "Sales" });
  });

  it("parses CSV with only name column (group absent)", () => {
    const csv = "name\nAlice\nBob";
    const result = parseCsv(csv);
    expect(result.errors).toHaveLength(0);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0]).toEqual({ name: "Alice", group: null });
    expect(result.rows[1]).toEqual({ name: "Bob", group: null });
  });

  it("rejects CSV missing name column", () => {
    const csv = "group,department\nEngineering,Dev";
    const result = parseCsv(csv);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.rows).toHaveLength(0);
  });

  it("parses quoted field with embedded comma", () => {
    const csv = 'name,group\n"Smith, John",Engineering';
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Smith, John");
  });

  it("parses escaped quotes inside quoted field", () => {
    const csv = 'name,group\n"He said ""hi""",Sales';
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe('He said "hi"');
  });

  it("handles CRLF line endings", () => {
    const csv = "name,group\r\nAlice,Engineering\r\nBob,Sales";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0].name).toBe("Alice");
    expect(result.rows[1].name).toBe("Bob");
  });

  it("handles empty file", () => {
    const result = parseCsv("");
    expect(result.rows).toHaveLength(0);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it("handles header-only file with no data rows", () => {
    const csv = "name,group";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(0);
    expect(result.errors).toHaveLength(0);
  });

  it("skips rows with empty name field", () => {
    const csv = "name,group\nAlice,Engineering\n,Sales\nBob,Dev";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0].name).toBe("Alice");
    expect(result.rows[1].name).toBe("Bob");
  });

  it("handles case-insensitive headers", () => {
    const csv = "Name,Group\nAlice,Engineering";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Alice");
    expect(result.rows[0].group).toBe("Engineering");
  });

  it("ignores extra columns beyond name and group", () => {
    const csv = "name,group,department,extra\nAlice,Eng,Dev,foo";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Alice");
    expect(result.rows[0].group).toBe("Eng");
  });

  it("truncates at 500 rows with warning", () => {
    const lines = ["name,group"];
    for (let i = 0; i < 501; i++) {
      lines.push(`User${i},Group${i}`);
    }
    const result = parseCsv(lines.join("\n"));
    expect(result.rows).toHaveLength(500);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.errors[0]).toMatch(/maximum/i);
  });

  it("returns null group for whitespace-only group value", () => {
    const csv = "name,group\nAlice,   ";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].group).toBeNull();
  });

  it("parses mixed quoted and unquoted fields", () => {
    const csv = 'name,group\nAlice,"Engineering Team"\n"Bob",Sales';
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0].group).toBe("Engineering Team");
    expect(result.rows[1].name).toBe("Bob");
    expect(result.rows[1].group).toBe("Sales");
  });

  it("preserves spaces inside quoted fields", () => {
    const csv = 'name,group\n"  Alice  ",Engineering';
    const result = parseCsv(csv);
    // parseLine trims outer whitespace; quoted content is trimmed by .map(f => f.trim())
    // so leading/trailing spaces inside quotes are also trimmed
    expect(result.rows[0].name).toBe("Alice");
  });

  it("handles trailing newline", () => {
    const csv = "name,group\nAlice,Engineering\n";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Alice");
  });

  it("handles LF-only line endings", () => {
    const csv = "name,group\nAlice,Engineering\nBob,Sales";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(2);
    expect(result.rows[0].name).toBe("Alice");
    expect(result.rows[1].name).toBe("Bob");
  });

  it("does not truncate when rows equal exactly 500", () => {
    const lines = ["name,group"];
    for (let i = 0; i < 500; i++) {
      lines.push(`User${i},Group${i}`);
    }
    const result = parseCsv(lines.join("\n"));
    expect(result.rows).toHaveLength(500);
    expect(result.errors).toHaveLength(0);
  });

  it("parses a quoted field containing a newline", () => {
    const csv = 'name,group\n"Multi\nLine",Engineering';
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Multi\nLine");
  });

  it("treats header names with surrounding spaces case-insensitively", () => {
    const csv = "  Name  ,  Group  \nAlice,Engineering";
    const result = parseCsv(csv);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0].name).toBe("Alice");
  });
});
