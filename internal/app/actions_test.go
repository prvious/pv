package app

import (
	"testing"
)

func TestRegisterAction(t *testing.T) {
	// Save the original actions map
	originalActions := actions
	defer func() { actions = originalActions }()

	// Reset actions map for isolated testing
	actions = make(map[string]func() error)

	// Test registering a new action
	testActionCalled := false
	testAction := func() error {
		testActionCalled = true
		return nil
	}

	RegisterAction("test:action", testAction)

	// Verify action was registered
	registeredActions := GetActions()
	if _, exists := registeredActions["test:action"]; !exists {
		t.Error("Action 'test:action' was not registered")
	}

	// Verify we can call the registered action
	if action, exists := registeredActions["test:action"]; exists {
		if err := action(); err != nil {
			t.Errorf("Action execution failed: %v", err)
		}
		if !testActionCalled {
			t.Error("Test action was not called")
		}
	}
}

func TestGetActions(t *testing.T) {
	// Save the original actions map
	originalActions := actions
	defer func() { actions = originalActions }()

	// Reset actions map for isolated testing
	actions = make(map[string]func() error)

	// Register multiple actions
	RegisterAction("action1", func() error { return nil })
	RegisterAction("action2", func() error { return nil })
	RegisterAction("action3", func() error { return nil })

	// Get all actions
	allActions := GetActions()

	// Verify count
	if len(allActions) != 3 {
		t.Errorf("Expected 3 actions, got %d", len(allActions))
	}

	// Verify each action exists
	expectedActions := []string{"action1", "action2", "action3"}
	for _, name := range expectedActions {
		if _, exists := allActions[name]; !exists {
			t.Errorf("Expected action '%s' not found", name)
		}
	}
}

func TestMultipleActionRegistrations(t *testing.T) {
	// Save the original actions map
	originalActions := actions
	defer func() { actions = originalActions }()

	// Reset actions map for isolated testing
	actions = make(map[string]func() error)

	// Test that registering the same action twice overwrites
	firstCalled := false
	secondCalled := false

	RegisterAction("test:duplicate", func() error {
		firstCalled = true
		return nil
	})

	RegisterAction("test:duplicate", func() error {
		secondCalled = true
		return nil
	})

	// Execute the action
	registeredActions := GetActions()
	if action, exists := registeredActions["test:duplicate"]; exists {
		action()
	}

	// Only the second action should have been called
	if firstCalled {
		t.Error("First action should have been overwritten")
	}
	if !secondCalled {
		t.Error("Second action should have been called")
	}
}
