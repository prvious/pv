package cmd

import (
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/prvious/pv/internal/config"
)

var highlightColor = lipgloss.ANSIColor(141) // Purple matching ui.Purple

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
	{"Run composer install", func(a *config.Automation) config.AutoMode { return a.ComposerInstall }, func(a *config.Automation, m config.AutoMode) { a.ComposerInstall = m }},
	{"Copy .env.example to .env", func(a *config.Automation) config.AutoMode { return a.CopyEnv }, func(a *config.Automation, m config.AutoMode) { a.CopyEnv = m }},
	{"Generate APP_KEY", func(a *config.Automation) config.AutoMode { return a.GenerateKey }, func(a *config.Automation, m config.AutoMode) { a.GenerateKey = m }},
	{"Set APP_URL to project domain", func(a *config.Automation) config.AutoMode { return a.SetAppURL }, func(a *config.Automation, m config.AutoMode) { a.SetAppURL = m }},
	{"Configure Laravel Octane", func(a *config.Automation) config.AutoMode { return a.InstallOctane }, func(a *config.Automation, m config.AutoMode) { a.InstallOctane = m }},
	{"Create project database", func(a *config.Automation) config.AutoMode { return a.CreateDatabase }, func(a *config.Automation, m config.AutoMode) { a.CreateDatabase = m }},
	{"Run database migrations", func(a *config.Automation) config.AutoMode { return a.RunMigrations }, func(a *config.Automation, m config.AutoMode) { a.RunMigrations = m }},
	{"Update .env when services change", func(a *config.Automation) config.AutoMode { return a.ServiceEnvUpdate }, func(a *config.Automation, m config.AutoMode) { a.ServiceEnvUpdate = m }},
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

	doc := strings.Builder{}

	// Render tabs.
	var renderedTabs []string
	for i, t := range m.tabs {
		isFirst := i == 0
		isLast := i == len(m.tabs)-1
		isActive := i == m.activeTab

		var style lipgloss.Style
		if isActive {
			style = setupActiveTab()
		} else {
			style = setupInactiveTab()
		}

		border, _, _, _, _ := style.GetBorder()
		if isFirst && isActive {
			border.BottomLeft = "│"
		} else if isFirst && !isActive {
			border.BottomLeft = "├"
		} else if isLast && isActive {
			border.BottomRight = "│"
		} else if isLast && !isActive {
			border.BottomRight = "┤"
		}
		style = style.Border(border)
		renderedTabs = append(renderedTabs, style.Render(t))
	}

	row := lipgloss.JoinHorizontal(lipgloss.Top, renderedTabs...)
	doc.WriteString(row)
	doc.WriteString("\n")

	// Render active tab content.
	var content string
	switch m.activeTab {
	case 0:
		content = renderSetupMultiSelect("Select which PHP versions to install:", m.phpOptions, m.phpCursor)
	case 1:
		content = renderSetupMultiSelect("Composer is always installed. Select additional tools:", m.toolOptions, m.toolCursor)
	case 2:
		content = renderSetupMultiSelect("Select backing services to set up:", m.svcOptions, m.svcCursor)
	case 3:
		content = renderSettingsTab(m.tld, m.tldCursor, m.editing, m.daemon, &m.automation, m.settingsCursor)
	}

	windowStyle := lipgloss.NewStyle().
		BorderForeground(highlightColor).
		Padding(1, 2).
		Border(lipgloss.NormalBorder()).
		UnsetBorderTop().
		Width(lipgloss.Width(row))

	doc.WriteString(windowStyle.Render(content))

	// Help bar.
	doc.WriteString("\n\n")
	helpStyle := lipgloss.NewStyle().Faint(true)
	switch {
	case m.editing:
		doc.WriteString(helpStyle.Render("type to edit • ←/→ move cursor • enter/esc stop editing"))
	case m.activeTab == len(m.tabs)-1 && m.settingsCursor == 0:
		doc.WriteString(helpStyle.Render("←/→ navigate • ↑/↓ move • e to edit TLD • enter confirm • esc quit"))
	case m.activeTab == len(m.tabs)-1 && m.settingsCursor == 1:
		doc.WriteString(helpStyle.Render("←/→ navigate • ↑/↓ move • space toggle • enter confirm • esc quit"))
	case m.activeTab == len(m.tabs)-1:
		doc.WriteString(helpStyle.Render("←/→ navigate • ↑/↓ move • space cycle • enter confirm • esc quit"))
	default:
		doc.WriteString(helpStyle.Render("←/→ navigate • ↑/↓ move • space toggle • enter confirm • esc quit"))
	}

	outer := lipgloss.NewStyle().Padding(1, 2)
	return tea.NewView(outer.Render(doc.String()))
}

func renderSetupMultiSelect(desc string, opts []selectOption, cursor int) string {
	var b strings.Builder
	b.WriteString(lipgloss.NewStyle().Faint(true).Render(desc))
	b.WriteString("\n\n")

	if len(opts) == 0 {
		b.WriteString(lipgloss.NewStyle().Faint(true).Render("  No options available"))
		return b.String()
	}

	cursorStyle := lipgloss.NewStyle().Foreground(highlightColor).Bold(true)
	greenStyle := lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(2))

	for i, opt := range opts {
		prefix := "  "
		if i == cursor {
			prefix = cursorStyle.Render("> ")
		}
		check := "[ ]"
		label := opt.label
		if opt.selected {
			check = greenStyle.Render("[x]")
			label = greenStyle.Render(opt.label)
		}
		b.WriteString(fmt.Sprintf("%s%s %s\n", prefix, check, label))
	}

	return b.String()
}

func renderSettingsTab(tld string, tldCursor int, editing bool, daemon bool, automation *config.Automation, settingsCursor int) string {
	var b strings.Builder
	cursorStyle := lipgloss.NewStyle().Foreground(highlightColor).Bold(true)
	faintStyle := lipgloss.NewStyle().Faint(true)
	greenStyle := lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(2))

	// --- TLD section ---
	b.WriteString(lipgloss.NewStyle().Bold(true).Foreground(highlightColor).Render("Domain"))
	b.WriteString("\n")
	b.WriteString(faintStyle.Render("Top-level domain for local sites"))
	b.WriteString("\n\n")

	prefix := "  "
	if settingsCursor == 0 {
		prefix = cursorStyle.Render("> ")
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
			b.WriteString(faintStyle.Render("  (press e to edit)"))
		}
	}

	// --- Daemon section ---
	b.WriteString("\n\n")
	b.WriteString(lipgloss.NewStyle().Bold(true).Foreground(highlightColor).Render("Daemon"))
	b.WriteString("\n")
	b.WriteString(faintStyle.Render("Start pv automatically on login"))
	b.WriteString("\n\n")

	prefix = "  "
	if settingsCursor == 1 {
		prefix = cursorStyle.Render("> ")
	}
	if daemon {
		b.WriteString(prefix + greenStyle.Render("true"))
	} else {
		b.WriteString(prefix + faintStyle.Render("false"))
	}

	// --- Automation section ---
	b.WriteString("\n\n")
	b.WriteString(lipgloss.NewStyle().Bold(true).Foreground(highlightColor).Render("Automation"))
	b.WriteString("\n")
	b.WriteString(faintStyle.Render("Configure which steps run during pv link"))
	b.WriteString("\n\n")

	yellowStyle := lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(3))

	// Find longest label for alignment.
	maxLen := 0
	for _, item := range automationItems {
		if len(item.label) > maxLen {
			maxLen = len(item.label)
		}
	}

	for i, item := range automationItems {
		prefix := "  "
		if settingsCursor == i+2 {
			prefix = cursorStyle.Render("> ")
		}

		mode := item.get(automation)
		padding := strings.Repeat(" ", maxLen-len(item.label)+2)

		var modeStr string
		switch mode {
		case config.AutoOn:
			modeStr = greenStyle.Render("true")
		case config.AutoAsk:
			modeStr = yellowStyle.Render("ask")
		case config.AutoOff:
			modeStr = faintStyle.Render("false")
		}

		b.WriteString(fmt.Sprintf("%s%s%s%s\n", prefix, item.label, padding, modeStr))
	}

	return b.String()
}

// Tab border helpers using normal (non-rounded) borders.

func setupTabBorder(left, middle, right string) lipgloss.Border {
	border := lipgloss.NormalBorder()
	border.BottomLeft = left
	border.Bottom = middle
	border.BottomRight = right
	return border
}

func setupActiveTab() lipgloss.Style {
	return lipgloss.NewStyle().
		Border(setupTabBorder("┘", " ", "└"), true).
		BorderForeground(highlightColor).
		Padding(0, 1).
		Bold(true)
}

func setupInactiveTab() lipgloss.Style {
	return lipgloss.NewStyle().
		Border(setupTabBorder("┴", "─", "┴"), true).
		BorderForeground(highlightColor).
		Padding(0, 1).
		Faint(true)
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
