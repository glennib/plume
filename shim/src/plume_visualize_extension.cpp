// plume_visualize: the VISUALIZE grammar shim for plume.
//
// A grammar extension for DuckDB v2's PEG parser, and therefore C++ built against one DuckDB
// version (a v2 C++ extension links DuckDB statically and loads only into that exact version).
// It adds the statement suffix
//
//     <query> VISUALIZE <target list>      (VISUALISE is accepted too)
//
// and desugars it into `SELECT <target list> FROM (<query>)`, so the chart functions of the
// plume core, a C API extension, do the rest. It also registers the session settings
// plume_viewer, plume_wait and plume_max_show, which the core's show() reads through its
// context, and loads the core when it is not loaded yet.
//
// The grammar is active on a connection after SET active_grammar_extensions = ['plume_visualize'].

#include "duckdb.hpp"
#include "duckdb/common/string_util.hpp"
#include "duckdb/main/config.hpp"
#include "duckdb/main/database.hpp"
#include "duckdb/main/extension/extension_loader.hpp"
#include "duckdb/main/extension_helper.hpp"
#include "duckdb/parser/grammar_extension.hpp"
#include "duckdb/parser/peg/parsed_grammar.hpp"
#include "duckdb/parser/peg/transformer/peg_transformer.hpp"
#include "duckdb/parser/query_node/select_node.hpp"
#include "duckdb/parser/statement/select_statement.hpp"
#include "duckdb/parser/tableref/subqueryref.hpp"

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace duckdb {
namespace {

const char *const GRAMMAR_NAME = "plume_visualize";
const char *const CORE_NAME = "plume";
const char *const CORE_FILE = "plume.duckdb_extension";

//===--------------------------------------------------------------------===//
// The grammar
//===--------------------------------------------------------------------===//

// The transform of the replaced rules
//
//     SelectStatement        <- SelectStatementInternal VisualizeClause?
//     ExplainSelectStatement <- SelectStatementInternal VisualizeClause?
//     VisualizeClause        <- VisualizeKeyword TargetList
//
// (DuckDB's own rules are the same without the clause, and EXPLAIN has its own copy.) Without the
// clause the statement passes through as DuckDB's own transform would pass it. With it, the
// statement becomes the FROM of a new SELECT whose select list is the clause's target list, so
// `<query> VISUALIZE chart()...` is `SELECT chart()... FROM (<query>)`.
unique_ptr<TransformResultValue> FinalizeSelectStatement(PEGTransformer &transformer, ParseResult &parse_result) {
	auto &list = parse_result.Cast<ListParseResult>();
	auto statement = transformer.Transform<unique_ptr<SelectStatement>>(list.GetChild(0));
	auto &clause = list.Child<OptionalParseResult>(1);
	if (clause.HasResult()) {
		auto &clause_list = clause.GetResult().Cast<ListParseResult>();
		auto targets = transformer.Transform<vector<unique_ptr<ParsedExpression>>>(clause_list.GetChild(1));
		auto node = make_uniq<SelectNode>();
		node->select_list = std::move(targets);
		node->from_table = make_uniq<SubqueryRef>(std::move(statement));
		statement = make_uniq<SelectStatement>();
		statement->node = std::move(node);
	}
	unique_ptr<SQLStatement> result = std::move(statement);
	return make_uniq<TypedTransformResult<unique_ptr<SQLStatement>>>(std::move(result));
}

unique_ptr<TransformProcess> StartSelectStatement(PEGTransformer &transformer, ParseResult &parse_result) {
	return make_uniq<FinalizeTransformProcess>(transformer, parse_result, FinalizeSelectStatement);
}

class VisualizeGrammar final : public GrammarExtension {
public:
	VisualizeGrammar()
	    : GrammarExtension(GRAMMAR_NAME, "<query> VISUALIZE <chart expression>, also spelt VISUALISE: "
	                                     "SELECT <chart expression> FROM (<query>), for plume charts") {
	}

	vector<GrammarChange> GetChanges() const override {
		vector<GrammarChange> changes;
		// Reserved, so that neither word can be an implicit alias (`SELECT x VISUALIZE ...` would
		// otherwise alias x) or a table alias.
		changes.push_back(GrammarChange::AddChoice("ReservedKeyword", "'VISUALIZE'"));
		changes.push_back(GrammarChange::AddChoice("ReservedKeyword", "'VISUALISE'"));
		changes.push_back(GrammarChange::AddRule("VisualizeKeyword <- 'VISUALIZE' / 'VISUALISE'"));
		changes.push_back(GrammarChange::AddRule("VisualizeClause <- VisualizeKeyword TargetList"));
		changes.push_back(GrammarChange::ReplaceRule("SelectStatement <- SelectStatementInternal VisualizeClause?",
		                                             StartSelectStatement));
		changes.push_back(GrammarChange::ReplaceRule(
		    "ExplainSelectStatement <- SelectStatementInternal VisualizeClause?", StartSelectStatement));
		return changes;
	}
};

//===--------------------------------------------------------------------===//
// The settings
//===--------------------------------------------------------------------===//

// Session (or global) defaults for show()'s named parameters. Each defaults to NULL, which means
// the process-wide setting the core keeps (plume_set, PLUME_VIEWER, PLUME_WAIT): a value set
// here takes precedence over that, and the named parameter over both.
//
// DuckDB stores no value for an option registered with a NULL default, and current_setting() then
// reports the option as unrecognized, so each is set to a typed NULL right after registration
// (RESET of a value set with SET GLOBAL clears it again, which is DuckDB's rule for such options).

void CheckViewer(ClientContext &, SetScope, Value &parameter) {
	if (parameter.IsNull()) {
		return;
	}
	auto viewer = StringUtil::Lower(parameter.ToString());
	if (viewer != "auto" && viewer != "terminal" && viewer != "window" && viewer != "browser") {
		throw InvalidInputException("plume_viewer: expected one of 'auto', 'terminal', 'window', 'browser', got '%s'",
		                            parameter.ToString());
	}
	parameter = Value(viewer);
}

void RegisterSetting(DBConfig &config, const char *name, const string &description, const LogicalType &type,
                     set_option_callback_t check = nullptr) {
	config.AddExtensionOption(name, description, type, Value(), check);
	config.SetOptionByName(name, Value(type));
}

void RegisterSettings(DatabaseInstance &db) {
	auto &config = DBConfig::GetConfig(db);
	RegisterSetting(config, "plume_viewer",
	                "The viewer show() uses when the call names none: auto, terminal, window or browser. NULL, "
	                "the default, means plume' process-wide setting.",
	                LogicalType::VARCHAR, CheckViewer);
	RegisterSetting(config, "plume_wait",
	                "Whether show() blocks until its window is closed, when the call does not say. NULL, the "
	                "default, means plume' process-wide setting.",
	                LogicalType::BOOLEAN);
	RegisterSetting(config, "plume_max_show",
	                "How many rows of one result show() displays at most; 0 shows nothing. NULL, the default, "
	                "means plume' process-wide setting.",
	                LogicalType::UBIGINT);
}

//===--------------------------------------------------------------------===//
// The core
//===--------------------------------------------------------------------===//

// The path of this shared library, or empty when the platform cannot say.
string OwnPath() {
#ifdef _WIN32
	HMODULE module = nullptr;
	if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
	                        reinterpret_cast<LPCWSTR>(&OwnPath), &module)) {
		return string();
	}
	wchar_t buffer[MAX_PATH];
	auto length = GetModuleFileNameW(module, buffer, MAX_PATH);
	if (length == 0 || length >= MAX_PATH) {
		return string();
	}
	auto size = WideCharToMultiByte(CP_UTF8, 0, buffer, static_cast<int>(length), nullptr, 0, nullptr, nullptr);
	string path(static_cast<size_t>(size), '\0');
	WideCharToMultiByte(CP_UTF8, 0, buffer, static_cast<int>(length), &path[0], size, nullptr, nullptr);
	return path;
#else
	Dl_info info;
	if (!dladdr(reinterpret_cast<void *>(&OwnPath), &info) || !info.dli_fname) {
		return string();
	}
	return string(info.dli_fname);
#endif
}

// Loads the plume core unless it is loaded already: the file next to this one, or the installed
// extension of that name. VISUALIZE is only sugar over the core's functions, so a shim without the
// core would fail every query it accepts.
void EnsureCoreLoaded(DatabaseInstance &db) {
	if (db.ExtensionIsLoaded(CORE_NAME)) {
		return;
	}
	auto &fs = db.GetFileSystem();
	string sibling;
	auto own = OwnPath();
	auto separator = own.find_last_of("/\\");
	if (separator != string::npos) {
		sibling = fs.JoinPath(own.substr(0, separator), CORE_FILE);
		if (fs.FileExists(sibling)) {
			ExtensionHelper::LoadExternalExtension(db, fs, ExtensionLoadOptions(sibling));
			return;
		}
	}
	try {
		ExtensionHelper::LoadExternalExtension(db, fs, ExtensionLoadOptions(CORE_NAME));
		return;
	} catch (std::exception &) {
		// not installed either
	}
	throw InvalidInputException("plume_visualize needs the plume extension, which is not loaded: LOAD it first, "
	                            "put %s next to %s, or INSTALL it (found neither%s nor an installed 'plume')",
	                            CORE_FILE, own.empty() ? "this extension" : own.c_str(),
	                            sibling.empty() ? "" : " " + sibling);
}

} // namespace
} // namespace duckdb

extern "C" {

DUCKDB_CPP_EXTENSION_ENTRY(plume_visualize, loader) {
	using namespace duckdb;
	auto &db = loader.GetDatabaseInstance();
	loader.SetDescription("The VISUALIZE clause for plume charts, and the plume_* settings");
	EnsureCoreLoaded(db);
	RegisterSettings(db);
	GrammarExtension::Register(db, make_shared_ptr<VisualizeGrammar>());
}
}
