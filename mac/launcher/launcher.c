/* ClaudeOS for Mac: the app's main program, ClaudeOS.app/Contents/MacOS/ClaudeOS.

   It runs ClaudeOS's Python inside this process instead of starting a separate Python program. That way
   macOS sees one app called ClaudeOS: its name in the Dock and the menu bar, and its permissions
   (microphone, Bluetooth, Accessibility), asked for with the reasons written in its Info.plist.

   ClaudeOS Setup (mac/setup.sh) writes ~/ClaudeOS/launch.conf:
       python=/Users/<you>/ClaudeOS/runtime/bin/python                    ClaudeOS's Python environment
       libpython=/Users/<you>/ClaudeOS/python/<version>/lib/libpython3.12.dylib
   It lives outside the app, so the app is the same on every Mac and is signed once, when it's built.

   Python finds that environment through PYTHONEXECUTABLE ("these variables imply that we should be using
   them as sys.executable and when searching for venvs", CPython's Modules/getpath.py), and ClaudeOS removes
   the variable again as it starts. -P keeps the home folder (where this starts) off Python's import path.

   Build: mac/launcher/build.sh (Zig's C compiler, for Apple silicon and Intel). Only the C library is used,
   so the same file also builds on Linux for the tests (tests/test_mac_launcher.py). */
#include <dlfcn.h>
#include <fcntl.h>
#include <limits.h>
#include <pwd.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

#define LINE_MAX_LEN 4096

/* Each buffer is big enough for what's put in it, so nothing is ever cut short. */
static char home_dir[PATH_MAX], co_home[PATH_MAX + 32], conf_python[LINE_MAX_LEN], conf_libpython[LINE_MAX_LEN];

/* Tells the person something went wrong: in the log, and on a Mac in a small window, so that starting
   ClaudeOS from the Dock is never silently nothing. */
static void tell(const char *msg) {
    fprintf(stderr, "ClaudeOS: %s\n", msg);
    fflush(stderr);
#ifdef __APPLE__
    char script[1024];
    snprintf(script, sizeof script, "display alert \"ClaudeOS couldn't start\" message \"%s\"", msg);
    char *args[] = {"/usr/bin/osascript", "-e", script, NULL};
    pid_t pid;
    if (posix_spawn(&pid, args[0], NULL, NULL, args, environ) == 0) {
        int status;
        waitpid(pid, &status, 0);
    }
#endif
}

/* The home folder: $HOME, or the account's record if that isn't set. */
static int find_home(void) {
    const char *h = getenv("HOME");
    if (!h || !*h) {
        struct passwd *pw = getpwuid(getuid());
        h = pw ? pw->pw_dir : NULL;
    }
    if (!h || !*h) return -1;
    snprintf(home_dir, sizeof home_dir, "%s", h);
    const char *co = getenv("CLAUDEOS_HOME");
    if (co && *co) snprintf(co_home, sizeof co_home, "%s", co);
    else snprintf(co_home, sizeof co_home, "%s/ClaudeOS", home_dir);
    return 0;
}

/* Reads launch.conf (key=value lines; anything else is ignored). */
static int read_conf(const char *path) {
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    char line[LINE_MAX_LEN];
    while (fgets(line, sizeof line, f)) {
        line[strcspn(line, "\r\n")] = '\0';
        char *eq = strchr(line, '=');
        if (!eq) continue;
        *eq = '\0';
        const char *value = eq + 1;
        if (strcmp(line, "python") == 0) snprintf(conf_python, sizeof conf_python, "%s", value);
        else if (strcmp(line, "libpython") == 0) snprintf(conf_libpython, sizeof conf_libpython, "%s", value);
    }
    fclose(f);
    return (conf_python[0] && conf_libpython[0]) ? 0 : -1;
}

/* Sends what ClaudeOS prints to ~/ClaudeOS/logs/app.log. */
static void log_to_file(void) {
    char dir[PATH_MAX + 64], file[PATH_MAX + 96];
    snprintf(dir, sizeof dir, "%s/logs", co_home);
    mkdir(dir, 0700);
    snprintf(file, sizeof file, "%s/app.log", dir);
    int fd = open(file, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (fd < 0) return;
    dup2(fd, STDOUT_FILENO);
    dup2(fd, STDERR_FILENO);
    if (fd > STDERR_FILENO) close(fd);
}

int main(void) {
    char conf[PATH_MAX + 64];
    if (find_home() != 0) {
        tell("It couldn't find your home folder.");
        return 1;
    }
    snprintf(conf, sizeof conf, "%s/launch.conf", co_home);
    if (read_conf(conf) != 0) {
        tell("ClaudeOS isn't set up on this Mac. Run ClaudeOS Setup and choose Install.");
        return 1;
    }
    log_to_file();
    if (access(conf_python, X_OK) != 0 || access(conf_libpython, R_OK) != 0) {
        tell("ClaudeOS's Python is missing from your ClaudeOS folder. Run ClaudeOS Setup and choose Install or Update.");
        return 1;
    }
    /* A Python of the person's own must not steer ClaudeOS's. */
    unsetenv("PYTHONHOME");
    unsetenv("PYTHONPATH");
    unsetenv("PYTHONSTARTUP");
    unsetenv("PYTHONINSPECT");
    setenv("PYTHONEXECUTABLE", conf_python, 1);
    setenv("CLAUDEOS_HOME", co_home, 0);
    if (chdir(home_dir) != 0) { /* staying where launchd started it is fine too */ }

    void *py = dlopen(conf_libpython, RTLD_NOW | RTLD_GLOBAL);
    if (!py) {
        fprintf(stderr, "ClaudeOS: %s\n", dlerror());
        tell("ClaudeOS's Python didn't load. Run ClaudeOS Setup and choose Install or Update.");
        return 1;
    }
    int (*py_main)(int, char **) = (int (*)(int, char **))dlsym(py, "Py_BytesMain");
    if (!py_main) {
        tell("ClaudeOS's Python is incomplete. Run ClaudeOS Setup and choose Install or Update.");
        return 1;
    }
    char *args[] = {conf_python, "-P", "-m", "claudeos", "run", NULL};
    return py_main(5, args);
}
