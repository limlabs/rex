// @ts-nocheck — CJS bridge, not checked by tsc (processed by rolldown/OXC only)
// React server bridge for Rex RSC bundles.
// Re-exports the react-server build AND adds missing client APIs
// (createContext, useState, useContext, etc.) that real-world libraries
// like PayloadCMS need but react-server omits.
//
// Uses CJS format so rolldown treats it as CJS and require() returns
// the module directly.

/* eslint-disable @typescript-eslint/no-explicit-any */

// Import the react-server build using the CJS file directly.
// The production CJS build is at react/cjs/react.react-server.production.js.
// We use require() which will be resolved by the require polyfill or rolldown.
const ReactServer = require('react-server-cjs-internal')

// Copy all server exports
const bridge: Record<string, any> = Object.assign({}, ReactServer)

// Add missing client APIs as minimal stubs

if (!bridge.createContext) {
    bridge.createContext = function createContext(defaultValue: any) {
        const context: any = {
            $$typeof: Symbol.for('react.context'),
            _currentValue: defaultValue,
            _currentValue2: defaultValue,
            _threadCount: 0,
            Provider: null,
            Consumer: null,
            _defaultValue: defaultValue,
            _globalName: null,
            displayName: undefined,
        }
        // Provider renders children passthrough — some libraries check
        // context.Provider !== context, so we use a distinct function.
        context.Provider = function ContextProvider(props: any) {
            return props.children
        }
        context.Provider.$$typeof = Symbol.for('react.provider')
        context.Provider._context = context
        context.Consumer = {
            $$typeof: Symbol.for('react.consumer'),
            _context: context,
        }
        return context
    }
}

if (!bridge.useState) {
    bridge.useState = function useState(initialState: any) {
        const value = typeof initialState === 'function' ? initialState() : initialState
        return [value, function () {}]
    }
}

if (!bridge.useReducer) {
    bridge.useReducer = function useReducer(_reducer: any, initialArg: any, init?: any) {
        const value = init ? init(initialArg) : initialArg
        return [value, function () {}]
    }
}

if (!bridge.useContext) {
    bridge.useContext = function useContext(context: any) {
        return context._currentValue
    }
}

if (!bridge.useRef) {
    bridge.useRef = function useRef(initialValue: any) {
        return { current: initialValue }
    }
}

if (!bridge.useEffect) {
    bridge.useEffect = function useEffect() {}
}

if (!bridge.useLayoutEffect) {
    bridge.useLayoutEffect = function useLayoutEffect() {}
}

if (!bridge.useCallback) {
    bridge.useCallback = function useCallback(callback: any) {
        return callback
    }
}

if (!bridge.useMemo) {
    bridge.useMemo = function useMemo(factory: any) {
        return factory()
    }
}

if (!bridge.useImperativeHandle) {
    bridge.useImperativeHandle = function useImperativeHandle() {}
}

if (!bridge.useInsertionEffect) {
    bridge.useInsertionEffect = function useInsertionEffect() {}
}

if (!bridge.useSyncExternalStore) {
    bridge.useSyncExternalStore = function useSyncExternalStore(
        _subscribe: any,
        getSnapshot: any,
    ) {
        return getSnapshot()
    }
}

if (!bridge.forwardRef) {
    bridge.forwardRef = function forwardRef(render: any) {
        return {
            $$typeof: Symbol.for('react.forward_ref'),
            render: render,
        }
    }
}

// Component class — not in react-server build, but needed by libraries like
// react-datepicker that extend React.Component in the server bundle.
if (!bridge.Component) {
    function Component(this: any, props: any, context: any) {
        this.props = props
        this.context = context
        this.refs = {}
    }
    Component.prototype.isReactComponent = {}
    Component.prototype.setState = function (_partialState: any, _callback: any) {}
    Component.prototype.forceUpdate = function (_callback: any) {}
    bridge.Component = Component
}

if (!bridge.PureComponent) {
    function PureComponent(this: any, props: any, context: any) {
        this.props = props
        this.context = context
        this.refs = {}
    }
    PureComponent.prototype = Object.create(bridge.Component.prototype)
    PureComponent.prototype.constructor = PureComponent
    PureComponent.prototype.isPureReactComponent = true
    bridge.PureComponent = PureComponent
}

// React 19 client internals — some packages access this
if (!bridge.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE) {
    bridge.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE = {
        H: null,
        A: null,
        T: null,
        S: null,
    }
}

module.exports = bridge
