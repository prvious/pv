package app

var actions = make(map[string]func() error)

func RegisterAction(name string, action func() error) {
	actions[name] = action
}

func GetActions() map[string]func() error {
	return actions
}
