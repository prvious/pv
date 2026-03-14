# Laravel Boost
- Laravel Boost is an MCP server that comes with powerful tools designed specifically for this application. Use them.

## Artisan Commands
- Run Artisan commands directly via the command line (e.g., ___SINGLE_BACKTICK___{{ $assist->artisanCommand('route:list') }}___SINGLE_BACKTICK___, ___SINGLE_BACKTICK___{{ $assist->artisanCommand('tinker --execute "..."') }}___SINGLE_BACKTICK___).
- Use ___SINGLE_BACKTICK___{{ $assist->artisanCommand('list') }}___SINGLE_BACKTICK___ to discover available commands and ___SINGLE_BACKTICK___{{ $assist->artisanCommand('[command] --help') }}___SINGLE_BACKTICK___ to check parameters.

## URLs
- Whenever you share a project URL with the user, you should use the ___SINGLE_BACKTICK___get-absolute-url___SINGLE_BACKTICK___ tool to ensure you're using the correct scheme, domain/IP, and port.

## Debugging
- Use the ___SINGLE_BACKTICK___database-query___SINGLE_BACKTICK___ tool when you only need to read from the database.
- Use the ___SINGLE_BACKTICK___database-schema___SINGLE_BACKTICK___ tool to inspect table structure before writing migrations or models.
- To execute PHP code for debugging, run ___SINGLE_BACKTICK___{{ $assist->artisanCommand('tinker --execute "your code here"') }}___SINGLE_BACKTICK___ directly.
- To read configuration values, read the config files directly or run ___SINGLE_BACKTICK___{{ $assist->artisanCommand('config:show [key]') }}___SINGLE_BACKTICK___.
- To inspect routes, run ___SINGLE_BACKTICK___{{ $assist->artisanCommand('route:list') }}___SINGLE_BACKTICK___ directly.
- To check environment variables, read the ___SINGLE_BACKTICK___.env___SINGLE_BACKTICK___ file directly.

@if (config('boost.browser_logs', false) !== false || config('boost.browser_logs_watcher', true) !== false)
## Reading Browser Logs With the ___SINGLE_BACKTICK___browser-logs___SINGLE_BACKTICK___ Tool
- You can read browser logs, errors, and exceptions using the ___SINGLE_BACKTICK___browser-logs___SINGLE_BACKTICK___ tool from Boost.
- Only recent browser logs will be useful - ignore old logs.
@endif

## Searching Documentation (Critically Important)
- Boost comes with a powerful ___SINGLE_BACKTICK___search-docs___SINGLE_BACKTICK___ tool you should use before trying other approaches when working with Laravel or Laravel ecosystem packages. This tool automatically passes a list of installed packages and their versions to the remote Boost API, so it returns only version-specific documentation for the user's circumstance. You should pass an array of packages to filter on if you know you need docs for particular packages.
- Search the documentation before making code changes to ensure we are taking the correct approach.
- Use multiple, broad, simple, topic-based queries at once. For example: ___SINGLE_BACKTICK___['rate limiting', 'routing rate limiting', 'routing']___SINGLE_BACKTICK___. The most relevant results will be returned first.
- Do not add package names to queries; package information is already shared. For example, use ___SINGLE_BACKTICK___test resource table___SINGLE_BACKTICK___, not ___SINGLE_BACKTICK___filament 4 test resource table___SINGLE_BACKTICK___.

### Available Search Syntax
1. Simple Word Searches with auto-stemming - query=authentication - finds 'authenticate' and 'auth'.
2. Multiple Words (AND Logic) - query=rate limit - finds knowledge containing both "rate" AND "limit".
3. Quoted Phrases (Exact Position) - query="infinite scroll" - words must be adjacent and in that order.
4. Mixed Queries - query=middleware "rate limit" - "middleware" AND exact phrase "rate limit".
5. Multiple Queries - queries=["authentication", "middleware"] - ANY of these terms.
