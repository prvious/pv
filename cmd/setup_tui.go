package cmd

import (
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/ui"
)

var (
	mutedColor = lipgloss.Color("#777777")
	rowBg      = lipgloss.Color("#212121")
)

type selectOption struct {
	label    string
	value    string
	selected bool
}

type setupModel struct {
	tabs      []string
	activeTab int
	width     int

	phpOptions  []selectOption
	phpCursor   int
	toolOptions []selectOption
	toolCursor  int
	svcOptions  []selectOption
	svcCursor   int

	tld            string
	tldCursor      int
	editing        bool // Whether the TLD input is in edit mode.
	daemon         bool
	automation     config.Automation
	settingsCursor int // Indexes: 0=TLD, 1=daemon, i+2=automationItems[i]

	confirmed bool
	quitting  bool
}

var automationItems = []struct {
	label string
	get   func(*config.Automation) config.AutoMode
	set   func(*config.Automation, config.AutoMode)
}{
	{"Auto install missing PHP version", func(a *config.Automation) config.AutoMode { return a.InstallPHPVersion }, func(a *config.Automation, m config.AutoMode) { a.InstallPHPVersion = m }},
	{"Set APP_URL to project domain", func(a *config.Automation) config.AutoMode { return a.SetAppURL }, func(a *config.Automation, m config.AutoMode) { a.SetAppURL = m }},
	{"Reset .env on service stop", func(a *config.Automation) config.AutoMode { return a.ServiceFallback }, func(a *config.Automation, m config.AutoMode) { a.ServiceFallback = m }},
}

func cycleAutoMode(m config.AutoMode) config.AutoMode {
	switch m {
	case config.AutoOn:
		return config.AutoAsk
	case config.AutoAsk:
		return config.AutoOff
	default:
		return config.AutoOn
	}
}

func newSetupModel(phpOpts, toolOpts, svcOpts []selectOption, tld string, daemon bool, automation config.Automation) setupModel {
	return setupModel{
		tabs:        []string{"PHP Versions", "Tools", "Services", "Settings"},
		phpOptions:  phpOpts,
		toolOptions: toolOpts,
		svcOptions:  svcOpts,
		tld:         tld,
		tldCursor:   len(tld),
		daemon:      daemon,
		automation:  automation,
		width:       80,
	}
}

func (m setupModel) Init() tea.Cmd {
	return nil
}

func (m setupModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		return m, nil

	case tea.KeyPressMsg:
		key := msg.String()

		// When editing the TLD input, capture all keys there.
		if m.editing {
			switch key {
			case "esc", "enter":
				m.editing = false
				return m, nil
			default:
				m.tld, m.tldCursor = handleTextInput(m.tld, m.tldCursor, key)
				return m, nil
			}
		}

		// Global keys (not editing).
		switch key {
		case "ctrl+c", "esc":
			m.quitting = true
			return m, tea.Quit
		case "enter":
			m.confirmed = true
			return m, tea.Quit
		}

		// Tab navigation — works everywhere.
		switch key {
		case "tab", "right", "l":
			m.activeTab = min(m.activeTab+1, len(m.tabs)-1)
			return m, nil
		case "shift+tab", "left", "h":
			m.activeTab = max(m.activeTab-1, 0)
			return m, nil
		}

		// Forward to active tab content.
		switch m.activeTab {
		case 0:
			m.phpOptions, m.phpCursor = handleMultiSelect(m.phpOptions, m.phpCursor, key)
		case 1:
			m.toolOptions, m.toolCursor = handleMultiSelect(m.toolOptions, m.toolCursor, key)
		case 2:
			m.svcOptions, m.svcCursor = handleMultiSelect(m.svcOptions, m.svcCursor, key)
		case 3:
			switch key {
			case "up", "k":
				if m.settingsCursor > 0 {
					m.settingsCursor--
				}
			case "down", "j":
				if m.settingsCursor < len(automationItems)+1 {
					m.settingsCursor++
				}
			case " ", "space", "x":
				if m.settingsCursor == 1 {
					m.daemon = !m.daemon
				} else if m.settingsCursor > 1 {
					idx := m.settingsCursor - 2
					current := automationItems[idx].get(&m.automation)
					automationItems[idx].set(&m.automation, cycleAutoMode(current))
				}
			case "e", "/", "i":
				if m.settingsCursor == 0 {
					m.editing = true
				}
			}
		}
	}

	return m, nil
}

func handleMultiSelect(opts []selectOption, cursor int, key string) ([]selectOption, int) {
	switch key {
	case "up", "k":
		if cursor > 0 {
			cursor--
		}
	case "down", "j":
		if cursor < len(opts)-1 {
			cursor++
		}
	case " ", "space", "x":
		if cursor >= 0 && cursor < len(opts) {
			opts[cursor].selected = !opts[cursor].selected
		}
	}
	return opts, cursor
}

func handleTextInput(value string, cursor int, key string) (string, int) {
	runes := []rune(value)
	switch key {
	case "left":
		if cursor > 0 {
			cursor--
		}
	case "right":
		if cursor < len(runes) {
			cursor++
		}
	case "backspace":
		if cursor > 0 {
			runes = append(runes[:cursor-1], runes[cursor:]...)
			cursor--
			value = string(runes)
		}
	case "delete":
		if cursor < len(runes) {
			runes = append(runes[:cursor], runes[cursor+1:]...)
			value = string(runes)
		}
	default:
		if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
			runes = append(runes[:cursor], append([]rune{rune(key[0])}, runes[cursor:]...)...)
			cursor++
			value = string(runes)
		}
	}
	return value, cursor
}

func (m setupModel) View() tea.View {
	if m.confirmed || m.quitting {
		return tea.NewView("")
	}

	contentWidth := m.width - 4
	if contentWidth < 40 {
		contentWidth = 40
	}

	doc := strings.Builder{}

	// Tab bar: inline text with │ separators.
	sep := lipgloss.NewStyle().Foreground(ui.AccentColor).Render("│")
	var tabs []string
	for i, t := range m.tabs {
		if i == m.activeTab {
			tabs = append(tabs, lipgloss.NewStyle().Bold(true).Render("  "+t+"  "))
		} else {
			tabs = append(tabs, lipgloss.NewStyle().Foreground(mutedColor).Render("  "+t+"  "))
		}
	}
	doc.WriteString(strings.Join(tabs, sep))
	doc.WriteString("\n")

	// Teal divider line.
	divLen := contentWidth
	if divLen > 72 {
		divLen = 72
	}
	divider := lipgloss.NewStyle().Foreground(ui.AccentColor).Render(strings.Repeat("─", divLen))
	doc.WriteString(divider)
	doc.WriteString("\n\n")

	// Tab content.
	switch m.activeTab {
	case 0:
		doc.WriteString(renderSetupMultiSelect("Select which PHP versions to install:", m.phpOptions, m.phpCursor))
	case 1:
		doc.WriteString(renderSetupMultiSelect("Composer is always installed. Select additional tools:", m.toolOptions, m.toolCursor))
	case 2:
		doc.WriteString(renderSetupMultiSelect("Select backing services to set up:", m.svcOptions, m.svcCursor))
	case 3:
		doc.WriteString(renderSettingsTab(m.tld, m.tldCursor, m.editing, m.daemon, &m.automation, m.settingsCursor))
	}

	// Help bar.
	doc.WriteString("\n")
	helpStyle := lipgloss.NewStyle().Foreground(mutedColor)
	switch {
	case m.editing:
		doc.WriteString(helpStyle.Render("type to edit • ←/→ move cursor • enter/esc stop editing"))
	case m.activeTab == len(m.tabs)-1 && m.settingsCursor == 0:
		doc.WriteString(helpStyle.Render("←/→ tab • ↑/↓ move • e to edit TLD • enter confirm • esc quit"))
	case m.activeTab == len(m.tabs)-1 && m.settingsCursor > 1:
		doc.WriteString(helpStyle.Render("←/→ tab • ↑/↓ move • space cycle • enter confirm • esc quit"))
	default:
		doc.WriteString(helpStyle.Render("←/→ tab • ↑/↓ move • space toggle • enter confirm • esc quit"))
	}

	outer := lipgloss.NewStyle().Padding(1, 2)
	return tea.NewView(outer.Render(doc.String()))
}

func renderSetupMultiSelect(desc string, opts []selectOption, cursor int) string {
	var b strings.Builder
	descStyle := lipgloss.NewStyle().Foreground(mutedColor)
	b.WriteString(descStyle.Render(desc))
	b.WriteString("\n\n")

	if len(opts) == 0 {
		b.WriteString(descStyle.Render("  No options available"))
		return b.String()
	}

	accentStyle := lipgloss.NewStyle().Foreground(ui.AccentColor)

	for i, opt := range opts {
		isCursor := i == cursor

		var prefix, indicator, label string

		if isCursor {
			prefix = accentStyle.Bold(true).Render("> ")
		} else {
			prefix = "  "
		}

		if opt.selected {
			indicator = accentStyle.Render("●")
		} else {
			indicator = "○"
		}

		if isCursor && opt.selected {
			label = accentStyle.Render(opt.label)
		} else {
			label = opt.label
		}

		b.WriteString(fmt.Sprintf("%s%s %s\n", prefix, indicator, label))

		// Gap between items.
		if i < len(opts)-1 {
			b.WriteString("\n")
		}
	}

	return b.String()
}

func renderSettingsTab(tld string, tldCursor int, editing bool, daemon bool, automation *config.Automation, settingsCursor int) string {
	var b strings.Builder
	accentBold := lipgloss.NewStyle().Foreground(ui.AccentColor).Bold(true)
	descStyle := lipgloss.NewStyle().Foreground(mutedColor)

	// --- TLD section ---
	b.WriteString(accentBold.Render("Domain"))
	b.WriteString("\n")
	b.WriteString(descStyle.Render("Top-level domain for local sites"))
	b.WriteString("\n\n")

	prefix := "  "
	if settingsCursor == 0 {
		prefix = accentBold.Render("> ")
	}

	if editing {
		runes := []rune(tld)
		cursorCharStyle := lipgloss.NewStyle().Reverse(true)
		var before, cursorChar, after string
		if tldCursor < len(runes) {
			before = string(runes[:tldCursor])
			cursorChar = cursorCharStyle.Render(string(runes[tldCursor]))
			after = string(runes[tldCursor+1:])
		} else {
			before = tld
			cursorChar = cursorCharStyle.Render(" ")
		}
		b.WriteString(prefix + before + cursorChar + after)
	} else {
		b.WriteString(prefix + tld)
		if settingsCursor == 0 {
			b.WriteString(descStyle.Render("  (press e to edit)"))
		}
	}

	// --- Daemon section ---
	b.WriteString("\n\n")
	b.WriteString(accentBold.Render("Daemon"))
	b.WriteString("\n")
	b.WriteString(descStyle.Render("Start pv automatically on login"))
	b.WriteString("\n\n")

	prefix = "  "
	if settingsCursor == 1 {
		prefix = accentBold.Render("> ")
	}
	if daemon {
		b.WriteString(prefix + ui.Accent.Render("true"))
	} else {
		b.WriteString(prefix + ui.Muted.Render("false"))
	}

	// --- Automation section ---
	b.WriteString("\n\n")
	b.WriteString(accentBold.Render("Automation"))
	b.WriteString("\n")
	b.WriteString(descStyle.Render("Configure which steps run during pv link"))
	b.WriteString("\n\n")

	// Find longest label for alignment.
	maxLen := 0
	for _, item := range automationItems {
		if len(item.label) > maxLen {
			maxLen = len(item.label)
		}
	}

	for i, item := range automationItems {
		prefix := "    "
		if settingsCursor == i+2 {
			prefix = "  " + accentBold.Render("> ")
		}

		mode := item.get(automation)
		padding := strings.Repeat(" ", maxLen-len(item.label)+2)

		b.WriteString(fmt.Sprintf("%s%s%s%s\n", prefix, item.label, padding, renderAutoToggle(mode)))
	}

	return b.String()
}

// renderAutoToggle renders a segmented pill toggle like: [true] ask  false
func renderAutoToggle(mode config.AutoMode) string {
	inactive := lipgloss.NewStyle().Padding(0, 1)
	activeOn := inactive.Background(ui.AccentColor).Foreground(lipgloss.Color("#000000"))
	activeAsk := inactive.Background(ui.OrangeColor).Foreground(lipgloss.Color("#000000"))
	activeFalse := inactive.Background(mutedColor).Foreground(lipgloss.Color("#000000"))

	trueStyle, askStyle, falseStyle := inactive, inactive, inactive
	switch mode {
	case config.AutoOn:
		trueStyle = activeOn
	case config.AutoAsk:
		askStyle = activeAsk
	default:
		falseStyle = activeFalse
	}

	return lipgloss.NewStyle().
		Background(rowBg).
		Render(trueStyle.Render("true") + askStyle.Render("ask") + falseStyle.Render("false"))
}

// Result extraction.

func selectedValues(opts []selectOption) []string {
	var result []string
	for _, opt := range opts {
		if opt.selected {
			result = append(result, opt.value)
		}
	}
	return result
}
