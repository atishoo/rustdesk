import 'dart:async';
import 'package:flutter/widgets.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/common/hbbs/hbbs.dart';
import 'package:flutter_hbb/common/widgets/peers_view.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_hbb/models/peer_model.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:get/get.dart';
import 'dart:convert';
import '../utils/http_service.dart' as http;

class GroupModel {
  final RxBool groupLoading = false.obs;
  final RxString groupLoadError = "".obs;
  final RxList<UserPayload> users = RxList.empty(growable: true);
  final RxList<Peer> peers = RxList.empty(growable: true);
  final RxString selectedAccessibleItemName = ''.obs;
  final RxString searchAccessibleItemNameText = ''.obs;
  WeakReference<FFI> parent;
  var initialized = false;
  var _cacheLoadOnceFlag = false;
  var _statusCode = 200;
  var _pulling = false;
  var _pullAgain = false;
  var _queuedPullQuiet = true;
  var _pullStale = false;
  List<String>? _pullContext;

  Future<bool> _pullIsCurrent() async {
    final context = _pullContext;
    if (context == null) return true;
    final current = await bind.mainAccountRequestContext();
    final valid = current[0] == context[0] &&
        current[1] == context[1] &&
        current[1].isNotEmpty;
    if (!valid) _pullStale = true;
    return valid;
  }

  final Map<String, VoidCallback> _peerIdUpdateListeners = {};

  bool get emtpy => users.isEmpty && peers.isEmpty;

  late final Peers peersModel;

  GroupModel(this.parent) {
    peersModel = Peers(
        name: PeersModelName.group,
        getInitPeers: () => peers,
        loadEvent: LoadEvent.group);
  }

  Future<void> pull({force = true, quiet = false}) async {
    if (bind.isDisableGroupPanel()) return;
    if (!gFFI.userModel.isLogin) return;
    if (gFFI.userModel.networkError.isNotEmpty) return;
    if (_pulling) {
      _pullAgain = true;
      _queuedPullQuiet = _queuedPullQuiet && quiet;
      return;
    }
    if (!force && initialized) return;
    _pulling = true;
    try {
      for (var round = 0; round < 2; round++) {
        _pullAgain = false;
        _queuedPullQuiet = true;
        _pullStale = false;
        _statusCode = 200;
        if (!quiet) {
          groupLoading.value = true;
          groupLoadError.value = "";
        }
        try {
          _pullContext = await bind.mainAccountRequestContext();
          if (_pullContext![1].isNotEmpty) {
            await _pull();
            if (await _pullIsCurrent()) {
              _tryHandlePullError();
              initialized = true;
              platformFFI.tryHandle({'name': LoadEvent.group});
              if (_statusCode == 401) {
                gFFI.userModel.refreshCurrentUser(reloadOtherModels: false);
              } else {
                _saveCache();
              }
            }
          } else {
            _pullStale = true;
          }
        } catch (_) {
          await _pullIsCurrent();
        } finally {
          _pullContext = null;
        }
        if (_pullStale) {
          initialized = false;
          groupLoadError.value = '';
          if (round == 0) {
            _pullAgain = true;
            _queuedPullQuiet = _queuedPullQuiet && quiet;
          }
        }
        if (!_pullAgain || round == 1) break;
        quiet = _queuedPullQuiet;
      }
    } finally {
      _pulling = false;
      groupLoading.value = false;
      if (_pullAgain) {
        final nextQuiet = _queuedPullQuiet;
        scheduleMicrotask(() => pull(force: true, quiet: nextQuiet));
      }
    }
  }

  Future<void> _pull() async {
    List<UserPayload> tmpUsers = List.empty(growable: true);
    if (!await _getUsers(tmpUsers)) {
      return;
    }
    List<Peer> tmpPeers = List.empty(growable: true);
    if (!await _getPeers(tmpPeers)) {
      return;
    }
    if (!await _pullIsCurrent()) return;
    // me first
    var index =
        tmpUsers.indexWhere((user) => user.email == gFFI.userModel.email.value);
    if (index != -1) {
      var user = tmpUsers.removeAt(index);
      tmpUsers.insert(0, user);
    }
    users.value = tmpUsers;
    if (!users.any((u) => u.email == selectedAccessibleItemName.value)) {
      selectedAccessibleItemName.value = '';
    }
    // recover online
    final oldOnlineIDs = peers.where((e) => e.online).map((e) => e.id).toList();
    peers.value = tmpPeers;
    peers
        .where((e) => oldOnlineIDs.contains(e.id))
        .map((e) => e.online = true)
        .toList();
    groupLoadError.value = '';
    _callbackPeerUpdate();
  }

  Future<bool> _getUsers(List<UserPayload> tmpUsers) async {
    final (apiServer, accountHeaders) = await getAccountHttpContext();
    final api = "$apiServer/api/users";
    try {
      var uri0 = Uri.parse(api);
      final pageSize = 100;
      var total = 0;
      int current = 0;
      do {
        current += 1;
        var uri = Uri(
            scheme: uri0.scheme,
            host: uri0.host,
            path: uri0.path,
            port: uri0.port,
            queryParameters: {
              'current': current.toString(),
              'pageSize': pageSize.toString(),
              'accessible': '',
              'status': '1',
            });
        final resp = await http.get(uri,
            headers: Map<String, String>.of(accountHeaders));
        _statusCode = resp.statusCode;
        Map<String, dynamic> json =
            _jsonDecodeResp(decode_http_response(resp), resp.statusCode);
        if (json.containsKey('error')) {
          if (json['error'] == 'Admin required!' ||
              json['error']
                  .toString()
                  .contains('ambiguous column name: status')) {
            throw translate('upgrade_capylink_server_pro_to_{1.1.10}_tip');
          } else {
            throw json['error'];
          }
        }
        if (resp.statusCode != 200) {
          throw 'HTTP ${resp.statusCode}';
        }
        if (json.containsKey('total')) {
          if (total == 0) total = json['total'];
          if (json.containsKey('data')) {
            final data = json['data'];
            if (data is List) {
              for (final user in data) {
                final u = UserPayload.fromJson(user);
                int index = tmpUsers.indexWhere((e) => e.email == u.email);
                if (index < 0) {
                  tmpUsers.add(u);
                } else {
                  tmpUsers[index] = u;
                }
              }
            }
          }
        }
      } while (current * pageSize < total);
      return true;
    } catch (err) {
      debugPrint('get accessible users: $err');
      groupLoadError.value =
          '${translate('pull_group_failed_tip')}: ${translate(err.toString())}';
    }
    return false;
  }

  Future<bool> _getPeers(List<Peer> tmpPeers) async {
    try {
      final (apiServer, accountHeaders) = await getAccountHttpContext();
      final api = "$apiServer/api/peers";
      var uri0 = Uri.parse(api);
      final pageSize = 100;
      var total = 0;
      int current = 0;
      do {
        current += 1;
        var queryParameters = {
          'current': current.toString(),
          'pageSize': pageSize.toString(),
          'accessible': '',
          'status': '1',
        };
        var uri = Uri(
            scheme: uri0.scheme,
            host: uri0.host,
            path: uri0.path,
            port: uri0.port,
            queryParameters: queryParameters);
        final resp = await http.get(uri,
            headers: Map<String, String>.of(accountHeaders));
        _statusCode = resp.statusCode;

        Map<String, dynamic> json =
            _jsonDecodeResp(decode_http_response(resp), resp.statusCode);
        if (json.containsKey('error')) {
          throw json['error'];
        }
        if (resp.statusCode != 200) {
          throw 'HTTP ${resp.statusCode}';
        }
        if (json.containsKey('total')) {
          if (total == 0) total = json['total'];
          if (json.containsKey('data')) {
            final data = json['data'];
            if (data is List) {
              for (final p in data) {
                final peerPayload = PeerPayload.fromJson(p);
                final peer = PeerPayload.toPeer(peerPayload);
                int index = tmpPeers.indexWhere((e) => e.id == peer.id);
                if (index < 0) {
                  tmpPeers.add(peer);
                } else {
                  tmpPeers[index] = peer;
                }
              }
            }
          }
        }
      } while (current * pageSize < total);
      return true;
    } catch (err) {
      debugPrint('get accessible peers: $err');
      groupLoadError.value =
          '${translate('pull_group_failed_tip')}: ${translate(err.toString())}';
    }
    return false;
  }

  Map<String, dynamic> _jsonDecodeResp(String body, int statusCode) {
    try {
      Map<String, dynamic> json = jsonDecode(body);
      return json;
    } catch (e) {
      final err = body.isNotEmpty && body.length < 128 ? body : e.toString();
      if (statusCode != 200) {
        throw 'HTTP $statusCode, $err';
      }
      throw err;
    }
  }

  void _saveCache() {
    try {
      final map = (<String, dynamic>{
        "access_token": bind.mainGetLocalOption(key: 'access_token'),
        "users": users.map((e) => e.toGroupCacheJson()).toList(),
        'peers': peers.map((e) => e.toGroupCacheJson()).toList()
      });
      bind.mainSaveGroup(json: jsonEncode(map));
    } catch (e) {
      debugPrint('group save:$e');
    }
  }

  Future<void> loadCache() async {
    try {
      if (_cacheLoadOnceFlag || groupLoading.value || initialized) return;
      _cacheLoadOnceFlag = true;
      final rawToken = bind.mainGetLocalOption(key: 'access_token');
      if (rawToken.isEmpty) return;
      final context = await bind.mainAccountRequestContext();
      final access_token = context[1];
      if (access_token != rawToken) return;
      final cache = await bind.mainLoadGroup();
      if (groupLoading.value) return;
      final currentContext = await bind.mainAccountRequestContext();
      if (currentContext[0] != context[0] ||
          currentContext[1] != access_token) {
        return;
      }
      final data = jsonDecode(cache);
      if (data == null || data['access_token'] != access_token) return;
      users.clear();
      peers.clear();
      if (data['users'] is List) {
        for (var u in data['users']) {
          users.add(UserPayload.fromJson(u));
        }
      }
      if (data['peers'] is List) {
        for (final peer in data['peers']) {
          peers.add(Peer.fromJson(peer));
        }
        _callbackPeerUpdate();
      }
    } catch (e) {
      debugPrint("load group cache: $e");
    }
  }

  reset() async {
    initialized = false;
    groupLoadError.value = '';
    users.clear();
    peers.clear();
    selectedAccessibleItemName.value = '';
    await bind.mainClearGroup();
  }

  void _callbackPeerUpdate() {
    for (var listener in _peerIdUpdateListeners.values) {
      listener();
    }
  }

  void addPeerUpdateListener(String key, VoidCallback listener) {
    _peerIdUpdateListeners[key] = listener;
  }

  void removePeerUpdateListener(String key) {
    _peerIdUpdateListeners.remove(key);
  }

  void _tryHandlePullError() {
    String errorMessage = groupLoadError.value;
    // The error message is "Retrieving accessible devices is disabled."
    if (errorMessage.toLowerCase().contains('disabled')) {
      users.clear();
      peers.clear();
    }
  }
}
