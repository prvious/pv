---
applyTo: "internals/actions/"
---

# PV CLI Action Creation Instructions

This document provides step-by-step instructions for AI agents and developers to create new actions in the PV CLI.

## 📋 Requirements Overview

Each action in PV CLI is a self-contained package that:

-   Lives in `internal/actions/`
-   Auto-registers itself using `init()` functions
-   Can include embedded template files using `go:embed`
-   Follows a consistent structure and naming convention

## 🏗️ Action Creation Process

### Step 1: Create the Action Package Directory

Create a new directory under `internal/actions/` for your action:

```bash
mkdir -p internal/actions/[action-name]/
```

**Example:**

```bash
mkdir -p internal/actions/nginx/
```

### Step 2: Create the Main Action File

Create `[action-name].go` with the following structure:

```go
package [action-name]

import (
	_ "embed"
	"fmt"
	"os"

	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
	// Import other action packages if needed for dependencies
	// "github.com/prvious/pv/internal/actions/docker"
)

//go:embed [template-file.ext]
var [templateName] []byte

func init() {
	app.RegisterAction("[Human Readable Action Name]", Setup)
}

func Setup() error {
	log.Info("Setting up [action description]")

	// Your action logic here
	// Example: Create files, directories, call other actions

	if err := os.WriteFile("[output-file]", [templateName], 0644); err != nil {
		return fmt.Errorf("failed to write [output-file]: %w", err)
	}

	log.Info("Successfully created [action description]")
	return nil
}
```

### Step 3: Create Template Files (if needed)

If your action needs template files, create them as plain files in the same directory. the files names should be prefixed with `.stub` to indicate they are templates.:

```bash
# Example template files
touch internal/actions/[action-name]/config.stub
touch internal/actions/[action-name]/dockerfile.stub
touch internal/actions/[action-name]/.env.stub
```

### Step 4: Add Import to main.go

Add your action package to the imports in `main.go`:

```go
import (
	"github.com/prvious/pv/internal/app"

	// Import action packages to trigger init() functions
	_ "github.com/prvious/pv/internal/actions/[action-name]"  // ADD THIS LINE
)
```

### Step 5: Test the Action

Build and test your new action:

```bash
go build -o bin/pv .
./bin/pv
```

Your action should appear in the TUI menu automatically.

## 📝 Action Examples

### Simple Action (No Dependencies)

```go
// internal/actions/git/git.go
package git

import (
	"os/exec"
	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

func init() {
	app.RegisterAction("Initialize Git Repository", Setup)
}

func Setup() error {
	log.Info("Initializing Git repository")

	cmd := exec.Command("git", "init")
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to initialize git: %w", err)
	}

	log.Info("Git repository initialized")
	return nil
}
```

### Action with Template File

```go
// internal/actions/nginx/nginx.go
package nginx

import (
	_ "embed"
	"fmt"
	"os"
	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

//go:embed nginx.conf.stub
var nginxConfigStub []byte

func init() {
	app.RegisterAction("Setup Nginx Configuration", Setup)
}

func Setup() error {
	log.Info("Setting up Nginx configuration")

	if err := os.WriteFile("nginx.conf", nginxConfigStub, 0644); err != nil {
		return fmt.Errorf("failed to write nginx.conf: %w", err)
	}

	log.Info("Nginx configuration created")
	return nil
}
```

### Action with Dependencies

```go
// internal/actions/fullstack/fullstack.go
package fullstack

import (
	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
	"github.com/prvious/pv/internal/actions/docker"
	"github.com/prvious/pv/internal/actions/laravel"
)

func init() {
	app.RegisterAction("Setup Full-Stack Environment", Setup)
}

func Setup() error {
	log.Info("Setting up full-stack development environment")

	// Call other actions
	if err := docker.SetupCompose(); err != nil {
		return fmt.Errorf("failed to setup Docker: %w", err)
	}

	if err := laravel.Setup(); err != nil {
		return fmt.Errorf("failed to setup Laravel: %w", err)
	}

	log.Info("Full-stack environment ready")
	return nil
}
```

## 🎯 Best Practices

### Naming Conventions

-   **Package name:** lowercase, single word (e.g., `docker`, `laravel`, `nginx`)
-   **Function name:** `Setup()` for main action function
-   **Action display name:** Human-readable with proper capitalization (e.g., "Setup Laravel Project")

### Error Handling

-   Always return descriptive errors with context
-   Use `fmt.Errorf()` with `%w` verb to wrap errors
-   Log progress with `log.Info()` for user feedback

### File Operations

-   Use `os.WriteFile()` for creating files
-   Use `os.MkdirAll()` for creating directories
-   Set appropriate file permissions (usually `0644` for files, `0755` for directories)

### Template Files

-   Keep template files in the same directory as the action
-   Use descriptive names with `.stub` extension
-   Embed with `//go:embed` directive

### Dependencies

-   Actions can call other actions by importing their packages
-   This creates reusable building blocks
-   Both actions remain visible in the menu independently

## 🔄 Auto-Discovery System

The PV CLI uses an auto-discovery system:

1. **Registration:** Each action registers itself using `app.RegisterAction()` in its `init()` function
2. **Discovery:** The TUI automatically discovers all registered actions
3. **Execution:** When selected, the corresponding action function is called
4. **No Manual Maintenance:** No need to manually update action lists

## ✅ Checklist for New Actions

-   [ ] Created package directory under `internal/actions/`
-   [ ] Created main action file with proper structure
-   [ ] Added `init()` function with `app.RegisterAction()` call
-   [ ] Created template files if needed with `go:embed` directives
-   [ ] Added package import to `main.go`
-   [ ] Tested action appears in TUI menu
-   [ ] Tested action executes successfully
-   [ ] Added appropriate error handling and logging
-   [ ] Followed naming conventions and best practices

## 🚀 Quick Template

Use this template to quickly create new actions:

```bash
# 1. Create directory
mkdir -p internal/actions/ACTIONNAME

# 2. Create main file
cat > internal/actions/ACTIONNAME/ACTIONNAME.go << 'EOF'
package ACTIONNAME

import (
	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

func init() {
	app.RegisterAction("Your Action Name", Setup)
}

func Setup() error {
	log.Info("Setting up your action")

	// Your action logic here

	log.Info("Action completed successfully")
	return nil
}
EOF

# 3. Add import to main.go (manually)
# 4. Build and test
go build -o bin/pv .
```

Replace `ACTIONNAME` with your actual action name and customize the logic as needed.
