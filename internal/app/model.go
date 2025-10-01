package app

import tea "github.com/charmbracelet/bubbletea"

type Model struct {
	searchInput  string
	options      []string
	filteredOpts []string
	selected     int
	width        int
	height       int
}

func InitialModel() Model {
	options := getAvailableActions()

	return Model{
		options:      options,
		filteredOpts: options,
		selected:     0,
	}
}

func getAvailableActions() []string {
	registeredActions := GetActions()
	actionNames := make([]string, 0, len(registeredActions))
	for actionName := range registeredActions {
		actionNames = append(actionNames, actionName)
	}
	return actionNames
}

func (m Model) Init() tea.Cmd {
	return nil
}
