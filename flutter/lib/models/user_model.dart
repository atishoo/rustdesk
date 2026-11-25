import 'dart:async';
import 'dart:convert';

import 'package:bot_toast/bot_toast.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hbb/common/hbbs/hbbs.dart';
import 'package:flutter_hbb/models/ab_model.dart';
import 'package:get/get.dart';

import '../common.dart';
import '../utils/http_service.dart' as http;
import 'model.dart';
import 'platform_model.dart';

bool refreshingUser = false;

class UserModel {
  final RxString email = ''.obs;
  final RxString nickname = ''.obs;
  final RxString avatar = ''.obs;
  final RxBool isAdmin = false.obs;
  final RxString networkError = ''.obs;
  // True when networkError carries a server-reported error rather than a
  // connectivity failure; netWorkErrorWidget hides the network tip then.
  final RxBool networkErrorFromServer = false.obs;
  bool get isLogin => email.isNotEmpty;
  String get displayLabel =>
      nickname.value.trim().isEmpty ? email.value : nickname.value;

  WeakReference<FFI> parent;
  int _loginGeneration = 0;
  int _serverChanges = 0;
  bool _refreshAgain = false;
  bool _verifyAgain = false;
  bool _verifyingOnly = false;
  Future<void>? _pendingAccountWrite;
  Future<void>? _pendingLogout;
  Future<void> _pendingServerConfig = Future.value();

  UserModel(this.parent) {
    email.listen((p0) {
      // When email becomes empty, show login button
      // When email becomes non-empty:
      //  For _updateLocalUserInfo, network error will be set later
      //  For login success, should clear network error
      networkError.value = '';
    });
  }

  void refreshCurrentUser({bool reloadOtherModels = true}) async {
    if (bind.isDisableAccount() || _serverChanges != 0) {
      return;
    }
    if (refreshingUser) {
      if (reloadOtherModels) {
        _refreshAgain = true;
      } else if (!_verifyingOnly) {
        _verifyAgain = true;
      }
      return;
    }
    refreshingUser = true;
    _verifyingOnly = !reloadOtherModels;
    networkError.value = '';
    networkErrorFromServer.value = false;
    final generation = _loginGeneration;
    bool sameGeneration() =>
        _serverChanges == 0 && _loginGeneration == generation;
    var url = '';
    var token = '';
    bool isCurrent() =>
        sameGeneration() &&
        bind.mainGetLocalOption(key: 'access_token') == token;
    var shouldUpdateOtherModels = false;
    try {
      final [requestUrl, boundToken] = await bind.mainAccountRequestContext();
      url = requestUrl;
      token = boundToken;
      if (token.isEmpty) {
        final rawToken = bind.mainGetLocalOption(key: 'access_token');
        final currentContext = await bind.mainAccountRequestContext();
        if (sameGeneration() &&
            bind.mainGetLocalOption(key: 'access_token') == rawToken &&
            currentContext[0] == url &&
            currentContext[1].isEmpty &&
            email.isNotEmpty) {
          email.value = '';
          nickname.value = '';
          avatar.value = '';
          isAdmin.value = false;
          await gFFI.abModel.reset();
          await gFFI.groupModel.reset();
        }
        return;
      }
      if (!isCurrent()) return;
      _updateLocalUserInfo();
      final body = {
        'id': await bind.mainGetMyId(),
        'uuid': await bind.mainGetUuid(),
      };
      if (!isCurrent()) return;
      final response = await http.post(Uri.parse('$url/api/currentUser'),
          headers: {
            'Content-Type': 'application/json',
            'Authorization': 'Bearer $token'
          },
          body: json.encode(body));
      if (!isCurrent()) return;
      final currentContext = await bind.mainAccountRequestContext();
      if (currentContext[0] != url || currentContext[1] != token) return;
      final status = response.statusCode;
      if (status == 401 || status == 400) {
        await _writeAccountState(isCurrent, () async {
          final currentContext = await bind.mainAccountRequestContext();
          if (currentContext[0] == url && currentContext[1] == token) {
            await reset(resetOther: status == 401);
          }
        });
        return;
      }
      final data = json.decode(decode_http_response(response));
      final error = data['error'];
      if (error != null) {
        // The only failure known to come from the server itself, so the
        // check-your-network tip does not apply. Flag before the message is
        // set in the catch below so rebuilds read a consistent pair.
        networkErrorFromServer.value = true;
        throw error;
      }

      final user = UserPayload.fromJson(data);
      await _writeAccountState(isCurrent, () => _parseAndUpdateUser(user));
      shouldUpdateOtherModels = true;
    } catch (e) {
      if (!isCurrent()) return;
      if (url.isNotEmpty) {
        final currentContext = await bind.mainAccountRequestContext();
        if (currentContext[0] != url || currentContext[1] != token) return;
      }
      debugPrint('Failed to refreshCurrentUser: $e');
      // Surface failures in the address book / group tabs, which offer a
      // retry. Anything not flagged above -- transport errors, non-JSON or
      // unexpected-schema bodies (e.g. a filter's block page) -- keeps the
      // check-your-network tip.
      if (networkError.value.isEmpty) {
        networkError.value = e.toString();
      }
    } finally {
      try {
        if (sameGeneration() && shouldUpdateOtherModels && reloadOtherModels) {
          await updateOtherModels();
        }
      } finally {
        refreshingUser = false;
        _verifyingOnly = false;
        if (_refreshAgain) {
          _refreshAgain = false;
          _verifyAgain = false;
          scheduleMicrotask(refreshCurrentUser);
        } else if (_verifyAgain) {
          _verifyAgain = false;
          scheduleMicrotask(() => refreshCurrentUser(reloadOtherModels: false));
        }
      }
    }
  }

  static Map<String, dynamic>? getLocalUserInfo() {
    final userInfo = bind.mainGetLocalOption(key: 'user_info');
    if (userInfo == '') {
      return null;
    }
    try {
      return json.decode(userInfo);
    } catch (e) {
      debugPrint('Failed to get local user info "$userInfo": $e');
    }
    return null;
  }

  _updateLocalUserInfo() {
    final userInfo = getLocalUserInfo();
    if (userInfo != null) {
      email.value = (userInfo['email'] ?? '').toString();
      nickname.value = (userInfo['nickname'] ?? '').toString();
      avatar.value = (userInfo['avatar'] ?? '').toString();
    }
  }

  Future<void> reset({bool resetOther = false}) async {
    await bind.mainSetLocalOption(key: 'access_token', value: '');
    await bind.mainSetLocalOption(key: 'user_info', value: '');
    if (resetOther) {
      await gFFI.abModel.reset();
      await gFFI.groupModel.reset();
    }
    email.value = '';
    nickname.value = '';
    avatar.value = '';
  }

  Future<void> _parseAndUpdateUser(UserPayload user) async {
    email.value = user.email;
    nickname.value = user.nickname;
    avatar.value = user.avatar;
    isAdmin.value = user.isAdmin;
    await Future.wait([
      bind.mainSetLocalOption(key: 'user_info', value: jsonEncode(user)),
      if (isWeb)
        bind.mainSetLocalOption(key: 'verifier', value: user.verifier ?? ''),
    ]);
  }

  // update ab and group status
  static Future<void> updateOtherModels() async {
    await Future.wait([
      gFFI.abModel.pullAb(force: ForcePullAb.listAndCurrent, quiet: false),
      gFFI.groupModel.pull()
    ]);
  }

  Future<void> logOut({String? apiServer}) {
    final active = _pendingLogout;
    if (active != null) return active;
    late final Future<void> pending;
    pending = _logOutOnce(apiServer: apiServer).whenComplete(() {
      if (identical(_pendingLogout, pending)) _pendingLogout = null;
    });
    _pendingLogout = pending;
    return pending;
  }

  Future<void> _logOutOnce({String? apiServer}) async {
    var cancelledToken = bind.mainGetLocalOption(key: 'access_token');
    _loginGeneration++;
    final tag = gFFI.dialogManager.showLoading(translate('Waiting'));
    var currentApi = '';
    var currentToken = '';
    var url = '';
    try {
      try {
        await _pendingAccountWrite;
      } catch (_) {}
      try {
        cancelledToken = await bind.mainAccountAuthCancel();
      } catch (_) {}
      final context = await bind.mainAccountRequestContext();
      currentApi = context[0];
      currentToken = context[1];
      url = apiServer ?? currentApi;
      final logoutId = currentToken.isNotEmpty ? await bind.mainGetMyId() : '';
      final logoutUuid =
          currentToken.isNotEmpty ? await bind.mainGetUuid() : '';
      final latest = await bind.mainAccountRequestContext();
      final sameAccount = url == currentApi &&
          latest[0] == currentApi &&
          latest[1] == currentToken &&
          bind.mainGetLocalOption(key: 'access_token') == cancelledToken;
      var cleared = false;
      if (sameAccount) {
        await _writeAccountState(
            () =>
                bind.mainGetLocalOption(key: 'access_token') == cancelledToken,
            () async {
          await reset(resetOther: true);
          cleared = true;
        });
      }
      if (cleared && currentToken.isNotEmpty) {
        await http
            .postAccountLogout(
                apiServer: url,
                token: currentToken,
                id: logoutId,
                uuid: logoutUuid)
            .timeout(Duration(seconds: 2));
      }
    } catch (e) {
      debugPrint("request /api/logout failed: err=$e");
    } finally {
      gFFI.dialogManager.dismissByTag(tag);
    }
  }

  Future<bool> saveServerConfig(Future<bool> Function() save) {
    final pending = _pendingServerConfig.then((_) => save());
    _pendingServerConfig =
        pending.then<void>((_) {}, onError: (Object _, StackTrace __) {});
    return pending;
  }

  Future<void> changeServer(Future<void> Function() change) async {
    _loginGeneration++;
    _serverChanges++;
    try {
      await _pendingAccountWrite;
      await change();
    } finally {
      _serverChanges--;
    }
  }

  Future<void> _writeAccountState(
      bool Function() isCurrent, Future<void> Function() write) async {
    while (_pendingAccountWrite != null) {
      await _pendingAccountWrite;
    }
    if (!isCurrent()) return;
    final pending = write();
    _pendingAccountWrite = pending;
    try {
      await pending;
    } finally {
      _pendingAccountWrite = null;
    }
  }

  /// throw [RequestException]
  Future<LoginResponse> login(LoginRequest loginRequest,
      {bool Function()? isAttemptActive}) async {
    if (isAttemptActive != null && !isAttemptActive()) {
      throw RequestException(0, 'Please try again');
    }
    final generation = ++_loginGeneration;
    bool isCurrent() =>
        _serverChanges == 0 &&
        _loginGeneration == generation &&
        (isAttemptActive?.call() ?? true);
    void checkCurrent() {
      if (!isCurrent()) {
        throw RequestException(0, 'Please try again');
      }
    }

    checkCurrent();
    final [url, deployment] = await bind.mainAccountLoginContext();
    checkCurrent();
    final previousToken = bind.mainGetLocalOption(key: 'access_token');
    Future<void> checkLoginContext() async {
      checkCurrent();
      final current = await bind.mainAccountLoginContext();
      checkCurrent();
      if (current[0] != url ||
          current[1] != deployment ||
          bind.mainGetLocalOption(key: 'access_token') != previousToken) {
        throw RequestException(0, 'Please try again');
      }
    }

    checkCurrent();
    final resp = await http.post(Uri.parse('$url/api/login'),
        body: jsonEncode(loginRequest.toJson()));
    await checkLoginContext();

    final Map<String, dynamic> body;
    try {
      body = jsonDecode(decode_http_response(resp));
    } catch (e) {
      debugPrint("login: jsonDecode resp body failed: ${e.toString()}");
      if (resp.statusCode != 200) {
        BotToast.showText(
            contentColor: Colors.red, text: 'HTTP ${resp.statusCode}');
      }
      rethrow;
    }
    if (resp.statusCode != 200) {
      throw RequestException(resp.statusCode, body['error'] ?? '');
    }
    if (body['error'] != null) {
      throw RequestException(0, body['error']);
    }

    final response = getLoginResponseFromAuthBody(body, updateUser: false);
    final token = response.access_token;
    if (response.type == HttpType.kAuthResTypeToken && token != null) {
      await _writeAccountState(isCurrent, () async {
        await checkLoginContext();
        await bind.mainSetLocalOption(
            key: isWeb ? 'access_token' : 'authenticated_session',
            value: isWeb ? token : jsonEncode([token, response.user?.id]),
            loginDeployment: deployment);
        final user = response.user;
        if (user != null) {
          await _parseAndUpdateUser(user);
        } else if (loginRequest.type == HttpType.kAuthReqTypeAccount) {
          await bind.mainSetLocalOption(key: 'user_info', value: '{}');
        }
      });
      checkCurrent();
    }
    return response;
  }

  LoginResponse getLoginResponseFromAuthBody(Map<String, dynamic> body,
      {bool updateUser = true}) {
    final LoginResponse loginResponse;
    try {
      loginResponse = LoginResponse.fromJson(body);
    } catch (e) {
      debugPrint("login: jsonDecode LoginResponse failed: ${e.toString()}");
      rethrow;
    }

    final isLogInDone = loginResponse.type == HttpType.kAuthResTypeToken &&
        loginResponse.access_token != null;
    if (updateUser && isLogInDone && loginResponse.user != null) {
      _parseAndUpdateUser(loginResponse.user!);
    }

    return loginResponse;
  }

  /// Throws on network failures, non-success responses, and invalid response
  /// data. Returns an empty list when no API server is configured or a
  /// successful response contains no third-party login options.
  static Future<List<dynamic>> queryOidcLoginOptions() async {
    final url = await bind.mainGetApiServer();
    if (url.trim().isEmpty) return [];
    final resp = await http.get(Uri.parse('$url/api/login-options'));
    const successStatusCodeStart = 200;
    const successStatusCodeEnd = 300;
    if (resp.statusCode < successStatusCodeStart ||
        resp.statusCode >= successStatusCodeEnd) {
      throw RequestException(
          resp.statusCode, resp.reasonPhrase ?? 'Request failed');
    }
    final List<String> ops = [];
    for (final item in jsonDecode(resp.body)) {
      ops.add(item as String);
    }
    for (final item in ops) {
      if (item.startsWith('common-oidc/')) {
        return jsonDecode(item.substring('common-oidc/'.length));
      }
    }
    return ops
        .where((item) => item.startsWith('oidc/'))
        .map((item) => {'name': item.substring('oidc/'.length)})
        .toList();
  }
}
