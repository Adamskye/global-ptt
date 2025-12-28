# Global Push-to-Talk for Linux

Simple GUI tool to enable push-to-talk for any microphone and for any application. Supports X11 and Wayland (with Global Shortcuts portal).

<img width="679" height="324" alt="Screenshot of the Global Push-to-Talk program" src="https://github.com/user-attachments/assets/bd9a249b-7967-4a9a-bd43-3c48090d75be" />

This application is a work-in-progress. 

## Usage

Select your microphone from the dropdown, then press "Enable". Then, whenever you want push-to-talk in an application, select the "Global Push-to-Talk Virtual Microphone", like shown below in Firefox.

<img width="400" height="159" alt="Selecting the global push-to-talk virtual microphone" src="https://github.com/user-attachments/assets/1c239ff8-0810-42b5-a1f7-3bed305005fa" />

## Hotkeys on Wayland

This application was originally created to test and demonstrate Wayland support in [tauri-apps/global-hotkey](https://github.com/tauri-apps/global-hotkey). The XDG GlobalShortcuts portal is required, which is supported by KDE, GNOME, and Hyprland (as of writing this). Reconfiguring the push-to-talk trigger is done in your system's settings.
