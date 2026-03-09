package cmd

import (
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
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

	tld       string
	tldCursor int
	editing   bool // Whether the TLD input is in edit mode.

	confirmed bool
	quitting  bool
}

func newSetupModel(phpOpts, toolOpts, svcOpts []selectOption, tld string) setupModel {
	return setupModel{
		tabs:        []string{"PHP Versions", "Tools", "Services", "Settings"},
		phpOptions:  phpOpts,
		toolOptions: toolOpts,
		svcOptions:  svcOpts,
		tld:         tld,
		tldCursor:   len(tld),
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
			case "esc":
				m.editing = false
				return m, nil
			case "enter":
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
			if key == "e" || key == "/" || key == "i" {
				m.editing = true
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
		content = renderSetupSettings("TLD", "Top-level domain for local sites", m.tld, m.tldCursor, m.editing)
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
	case m.activeTab == len(m.tabs)-1:
		doc.WriteString(helpStyle.Render("←/→ navigate • e to edit TLD • enter confirm • esc quit"))
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

func renderSetupSettings(label, desc, value string, cursor int, editing bool) string {
	var b strings.Builder
	b.WriteString(lipgloss.NewStyle().Bold(true).Foreground(highlightColor).Render(label))
	b.WriteString("\n")
	b.WriteString(lipgloss.NewStyle().Faint(true).Render(desc))
	b.WriteString("\n\n")

	if !editing {
		// Show value without cursor.
		b.WriteString("> " + value)
		b.WriteString("\n\n")
		b.WriteString(lipgloss.NewStyle().Faint(true).Render("Press e to edit"))
		return b.String()
	}

	// Show value with cursor.
	runes := []rune(value)
	cursorCharStyle := lipgloss.NewStyle().Reverse(true)

	var before, cursorChar, after string
	if cursor < len(runes) {
		before = string(runes[:cursor])
		cursorChar = cursorCharStyle.Render(string(runes[cursor]))
		after = string(runes[cursor+1:])
	} else {
		before = value
		cursorChar = cursorCharStyle.Render(" ")
		after = ""
	}

	b.WriteString("> " + before + cursorChar + after)
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

func (m setupModel) selectedPHPValues() []string {
	var result []string
	for _, opt := range m.phpOptions {
		if opt.selected {
			result = append(result, opt.value)
		}
	}
	return result
}

func (m setupModel) selectedToolValues() []string {
	var result []string
	for _, opt := range m.toolOptions {
		if opt.selected {
			result = append(result, opt.value)
		}
	}
	return result
}

func (m setupModel) selectedServiceValues() []string {
	var result []string
	for _, opt := range m.svcOptions {
		if opt.selected {
			result = append(result, opt.value)
		}
	}
	return result
}
