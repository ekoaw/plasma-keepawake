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
    property var rules: [] // [{name, enabled, value, error, expr}]
    property string actionError: ""
    // cmd string -> callback(ok, stdout), for calls whose own result (not
    // just the refreshed status) matters - AddRule/UpdateRule/RemoveRule.
    property var pendingCallbacks: ({})

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
                return
            }

            const callback = root.pendingCallbacks[sourceName]
            if (callback) {
                delete root.pendingCallbacks[sourceName]
                callback(exitCode === 0, stdout)
            }
            // Always refresh too, so the rule list reflects whatever
            // actually happened rather than what the caller assumed would.
            root.refresh()
        }

        function run(cmd) {
            connectSource(cmd)
        }
    }

    function refresh() {
        exec.run(statusCmd)
    }

    // Runs a daemon method that returns "(b success, s error)" and reports
    // the result back via onResult(success, error).
    function runAction(cmd, onResult) {
        pendingCallbacks[cmd] = (ok, stdout) => {
            if (!ok) {
                onResult(false, "command failed to run")
                return
            }
            try {
                const d = JSON.parse(stdout)["data"]
                onResult(d[0], d[1])
            } catch (e) {
                onResult(false, "couldn't parse daemon response: " + e)
            }
        }
        exec.run(cmd)
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
                expr: r[4],
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

    function daemonCall(method, argSig, args) {
        return "busctl --user --json=short call " + busName + " " + objectPath + " " + busName
            + " " + method + (argSig.length > 0 ? " " + argSig : "") + " " + args.join(" ")
    }

    // Each takes onResult(success, error) - error is "" on success.
    function addRule(name, expr, enabled, onResult) {
        runAction(daemonCall("AddRule", "ssb",
            [shellQuote(name), shellQuote(expr), enabled ? "true" : "false"]), onResult)
    }

    function updateRule(name, expr, onResult) {
        runAction(daemonCall("UpdateRule", "ss", [shellQuote(name), shellQuote(expr)]), onResult)
    }

    function removeRule(name, onResult) {
        runAction(daemonCall("RemoveRule", "s", [shellQuote(name)]), onResult)
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

        PlasmaComponents.Label {
            visible: root.actionError.length > 0
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.negativeTextColor
            text: root.actionError
        }

        Repeater {
            model: root.daemonRunning ? root.rules : []
            delegate: ColumnLayout {
                id: ruleDelegate
                required property var modelData
                property bool editing: false
                Layout.fillWidth: true
                spacing: 0

                RowLayout {
                    Layout.fillWidth: true
                    visible: !ruleDelegate.editing

                    QQC2.CheckBox {
                        checked: ruleDelegate.modelData.enabled
                        onToggled: root.setRuleEnabled(ruleDelegate.modelData.name, checked)
                    }
                    PlasmaComponents.Label {
                        Layout.fillWidth: true
                        text: ruleDelegate.modelData.name
                        elide: Text.ElideRight
                    }
                    Kirigami.Icon {
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                        source: ruleDelegate.modelData.error.length > 0
                            ? "data-error"
                            : (ruleDelegate.modelData.value ? "emblem-checked" : "emblem-question")
                        QQC2.ToolTip.text: ruleDelegate.modelData.error.length > 0
                            ? ruleDelegate.modelData.error
                            : (ruleDelegate.modelData.value ? "currently true" : "currently false")
                        QQC2.ToolTip.visible: iconMouse.containsMouse
                        MouseArea {
                            id: iconMouse
                            anchors.fill: parent
                            hoverEnabled: true
                        }
                    }
                    PlasmaComponents.ToolButton {
                        icon.name: "document-edit"
                        display: QQC2.AbstractButton.IconOnly
                        onClicked: {
                            exprField.text = ruleDelegate.modelData.expr
                            ruleDelegate.editing = true
                        }
                    }
                    PlasmaComponents.ToolButton {
                        icon.name: "edit-delete"
                        display: QQC2.AbstractButton.IconOnly
                        onClicked: root.removeRule(ruleDelegate.modelData.name, (ok, error) => {
                            root.actionError = ok ? "" : ("Couldn't remove rule: " + error)
                        })
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: ruleDelegate.editing

                    QQC2.TextField {
                        id: exprField
                        Layout.fillWidth: true
                        font.family: "monospace"
                        onAccepted: saveButton.clicked()
                    }
                    PlasmaComponents.ToolButton {
                        id: saveButton
                        icon.name: "dialog-ok"
                        display: QQC2.AbstractButton.IconOnly
                        onClicked: root.updateRule(ruleDelegate.modelData.name, exprField.text, (ok, error) => {
                            root.actionError = ok ? "" : ("Couldn't update rule: " + error)
                            if (ok) ruleDelegate.editing = false
                        })
                    }
                    PlasmaComponents.ToolButton {
                        icon.name: "dialog-cancel"
                        display: QQC2.AbstractButton.IconOnly
                        onClicked: ruleDelegate.editing = false
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

        RowLayout {
            Layout.fillWidth: true
            visible: !addRuleForm.visible

            PlasmaComponents.Button {
                text: "Add rule"
                enabled: root.daemonRunning
                onClicked: addRuleForm.visible = true
            }
            Item { Layout.fillWidth: true }
            PlasmaComponents.Button {
                text: "Reload config"
                enabled: root.daemonRunning
                onClicked: root.reloadConfig()
            }
        }

        ColumnLayout {
            id: addRuleForm
            visible: false
            Layout.fillWidth: true

            QQC2.TextField {
                id: newRuleName
                Layout.fillWidth: true
                placeholderText: "Rule name"
            }
            QQC2.TextField {
                id: newRuleExpr
                Layout.fillWidth: true
                placeholderText: "Rhai expression, e.g. mpris_playing(\"cliamp\")"
                font.family: "monospace"
            }
            RowLayout {
                Layout.fillWidth: true
                PlasmaComponents.Button {
                    text: "Add"
                    enabled: newRuleName.text.length > 0 && newRuleExpr.text.length > 0
                    onClicked: root.addRule(newRuleName.text, newRuleExpr.text, true, (ok, error) => {
                        if (ok) {
                            root.actionError = ""
                            newRuleName.text = ""
                            newRuleExpr.text = ""
                            addRuleForm.visible = false
                        } else {
                            root.actionError = "Couldn't add rule: " + error
                        }
                    })
                }
                PlasmaComponents.Button {
                    text: "Cancel"
                    onClicked: {
                        newRuleName.text = ""
                        newRuleExpr.text = ""
                        addRuleForm.visible = false
                    }
                }
            }
        }
    }
}
