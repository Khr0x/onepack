// Generates the NuGet version reference corpus (ADR-008).
// Usage: dotnet run -- <inputs.json> <corpus.json>
using System.Text.Json;
using NuGet.Versioning;

var inputs = JsonSerializer.Deserialize<string[]>(File.ReadAllText(args[0]))!;

var parsed = inputs
    .Select(i => (Input: i, Ok: NuGetVersion.TryParse(i, out var v), Version: v))
    .ToList();

var valid = parsed.Where(p => p.Ok).Select(p => p.Version!).ToList();

// rank: number of versions strictly lower according to VersionComparer.Default.Compare.
// identity: index of the first equal version according to VersionComparer.Default.Equals.
// They are recorded separately because NuGet does not always make them match (e.g. "1.0.0-0" and "1.0.0--0").
var cases = parsed.Select(p => p.Ok
    ? new Case(p.Input, true, p.Version!.ToNormalizedString(), p.Version.ToFullString(), p.Version.IsPrerelease,
        p.Version.IsSemVer2,
        valid.Count(o => VersionComparer.Default.Compare(o, p.Version) < 0),
        valid.FindIndex(o => VersionComparer.Default.Equals(o, p.Version)))
    : new Case(p.Input, false, null, null, null, null, null, null));

var options = new JsonSerializerOptions
{
    WriteIndented = true,
    PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
    Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
};
var json = JsonSerializer.Serialize(new Corpus(typeof(NuGetVersion).Assembly.GetName().Version!.ToString(), cases), options);
File.WriteAllText(args[1], json + "\n");

record Corpus(string NugetVersioning, IEnumerable<Case> Cases);
record Case(string Input, bool Valid, string? Normalized, string? Full, bool? Prerelease, bool? Semver2, int? Rank, int? Identity);
