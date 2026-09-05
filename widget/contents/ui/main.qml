import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2

import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasma5support as P5Support
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.extras as PlasmaExtras
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    // Everything below is read from / written to the daemon's own D-Bus
    // service (org.plasmakeepawake.Daemon1) via `busctl`, run through the
    // plasma5support "executable" data engine - there is no generic
    // D-Bus-from-pure-QML binding in Plasma 6 (confirmed by checking how
    // KDE Connect's own widget does it: a compiled C++ QML plugin). Using
    // busctl --json=short instead of parsing plain text output keeps this
    // a plain KPackage with no C++/CMake build step. See PLAN.md.

    readonly property string busName: "org.plasmakeepawake.Daemon1"
    readonly property string objectPath: "/org/plasmakeepawake/Daemon1"
    readonly property string statusCmd: "busctl --user --json=short call " + busName + " " + objectPath
        + " org.freedesktop.DBus.Properties GetAll s " + busName

    property bool daemonRunning: false
    property bool inhibiting: false
    property string reason: ""
    property string reloadError: ""
    property var rules: [] // [{name, enabled, value, error}]

    Plasmoid.status: inhibiting ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus
    toolTipMainText: daemonRunning
        ? (inhibiting ? "Sleep inhibited" : "Sleep allowed")
        : "plasma-keepawaked not running"
    toolTipSubText: daemonRunning ? reason : ""

    P5Support.DataSource {
        id: exec
        engine: "executable"
        connectedSources: []

        onNewData: (sourceName, data) => {
            disconnectSource(sourceName)
            const exitCode = data["exit code"]
            const stdout = data["stdout"] || ""

            if (sourceName === root.statusCmd) {
                root.applyStatus(exitCode === 0, stdout)
            } else {
                // A SetRuleEnabled/ReloadConfig call finished; refresh
                // regardless of its own result so the UI reflects whatever
                // actually happened rather than what we assumed would.
                root.refresh()
            }
        }

        function run(cmd) {
            connectSource(cmd)
        }
    }

    function refresh() {
        exec.run(statusCmd)
    }

    function applyStatus(ok, stdout) {
        if (!ok || stdout.length === 0) {
            daemonRunning = false
            inhibiting = false
            reason = ""
            rules = []
            return
        }
        try {
            // busctl --json=short GetAll: {"type":"a{sv}","data":[{ "Rules": {...}, "Inhibiting": {...}, ... }]}
            const props = JSON.parse(stdout)["data"][0]
            daemonRunning = true
            inhibiting = props["Inhibiting"]["data"]
            reason = props["Reason"]["data"]
            reloadError = props["ReloadError"]["data"]
            rules = props["Rules"]["data"].map(r => ({
                name: r[0],
                enabled: r[1],
                value: r[2],
                error: r[3],
            }))
        } catch (e) {
            console.warn("plasma-keepawake widget: couldn't parse daemon status:", e, stdout)
            daemonRunning = false
        }
    }

    function shellQuote(s) {
        return "'" + String(s).replace(/'/g, "'\\''") + "'"
    }

    function setRuleEnabled(name, enabled) {
        exec.run("busctl --user --json=short call " + busName + " " + objectPath + " " + busName
            + " SetRuleEnabled sb " + shellQuote(name) + " " + (enabled ? "true" : "false"))
    }

    function reloadConfig() {
        exec.run("busctl --user --json=short call " + busName + " " + objectPath + " " + busName + " ReloadConfig")
    }

    Timer {
        interval: 3000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    compactRepresentation: Kirigami.Icon {
        source: "preferences-system-power-management"
        active: mouseArea.containsMouse

        MouseArea {
            id: mouseArea
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.expanded = !root.expanded
        }
    }

    fullRepresentation: ColumnLayout {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 20
        Layout.preferredHeight: implicitHeight
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            Kirigami.Icon {
                source: root.inhibiting ? "media-playback-start" : "media-playback-pause"
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
            }
            PlasmaExtras.Heading {
                level: 3
                Layout.fillWidth: true
                text: !root.daemonRunning
                    ? "plasma-keepawaked not running"
                    : (root.inhibiting ? "Sleep inhibited" : "Sleep allowed")
            }
        }

        PlasmaComponents.Label {
            visible: root.daemonRunning && root.reason.length > 0
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "Because: " + root.reason
        }

        PlasmaComponents.Label {
            visible: root.reloadError.length > 0
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.negativeTextColor
            text: "Config reload failed, showing last-good rules: " + root.reloadError
        }

        Kirigami.Separator {
            Layout.fillWidth: true
            visible: root.daemonRunning
        }

        Repeater {
            model: root.daemonRunning ? root.rules : []
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true

                QQC2.CheckBox {
                    checked: modelData.enabled
                    onToggled: root.setRuleEnabled(modelData.name, checked)
                }
                PlasmaComponents.Label {
                    Layout.fillWidth: true
                    text: modelData.name
                    elide: Text.ElideRight
                }
                Kirigami.Icon {
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    source: modelData.error.length > 0
                        ? "data-error"
                        : (modelData.value ? "emblem-checked" : "emblem-question")
                    QQC2.ToolTip.text: modelData.error.length > 0 ? modelData.error : (modelData.value ? "currently true" : "currently false")
                    QQC2.ToolTip.visible: iconMouse.containsMouse
                    MouseArea {
                        id: iconMouse
                        anchors.fill: parent
                        hoverEnabled: true
                    }
                }
            }
        }

        PlasmaExtras.PlaceholderMessage {
            visible: root.daemonRunning && root.rules.length === 0
            Layout.fillWidth: true
            text: "No rules configured"
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        PlasmaComponents.Button {
            text: "Reload config"
            enabled: root.daemonRunning
            onClicked: root.reloadConfig()
        }
    }
}
