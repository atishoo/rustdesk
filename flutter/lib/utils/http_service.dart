import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter_hbb/consts.dart';
import 'package:http/http.dart' as http;
import '../models/platform_model.dart';
import 'package:flutter_hbb/common.dart';
export 'package:http/http.dart' show Response;

enum HttpMethod { get, post, put, delete }

class HttpService {
  Future<http.Response> sendRequest(
    Uri url,
    HttpMethod method, {
    Map<String, String>? headers,
    dynamic body,
  }) =>
      _sendRequest(url, method, headers: headers, body: body);

  Future<http.Response> _sendRequest(
    Uri url,
    HttpMethod method, {
    Map<String, String>? headers,
    dynamic body,
    (String, String)? logoutContext,
  }) async {
    headers ??= {'Content-Type': 'application/json'};

    final authorization = headers['Authorization'];
    String? requestApi;
    String? requestToken;
    if (logoutContext != null) {
      final (apiServer, token) = logoutContext;
      final api = Uri.tryParse(apiServer);
      if (method != HttpMethod.post ||
          token.isEmpty ||
          authorization != 'Bearer $token' ||
          api == null ||
          (api.scheme != 'http' && api.scheme != 'https') ||
          api.host.isEmpty ||
          api.userInfo.isNotEmpty ||
          api.query.isNotEmpty ||
          api.fragment.isNotEmpty ||
          url != Uri.parse('$apiServer/api/logout')) {
        throw http.ClientException('Invalid logout request', url);
      }
    } else if (authorization != null && authorization.startsWith('Bearer ')) {
      final [apiServer, boundToken] = await bind.mainAccountRequestContext();
      final api = Uri.tryParse(apiServer);
      final basePath =
          api?.normalizePath().path.replaceFirst(RegExp(r'/$'), '') ?? '';
      if (boundToken.isEmpty ||
          authorization != 'Bearer $boundToken' ||
          api == null ||
          (api.scheme != 'http' && api.scheme != 'https') ||
          api.userInfo.isNotEmpty ||
          url.userInfo.isNotEmpty ||
          url.scheme != api.scheme ||
          url.host != api.host ||
          url.port != api.port ||
          !url.normalizePath().path.startsWith('$basePath/api/')) {
        throw http.ClientException('Account session changed', url);
      }
      requestApi = apiServer;
      requestToken = boundToken;
    }

    // Use Rust HTTP implementation for non-web platforms for consistency.
    var useFlutterHttp = (isWeb || kIsWeb);
    if (!useFlutterHttp) {
      final enableFlutterHttpOnRust =
          mainGetLocalBoolOptionSync(kOptionEnableFlutterHttpOnRust);
      // Use flutter http if:
      // Not `enableFlutterHttpOnRust` and no proxy is set
      useFlutterHttp =
          !(enableFlutterHttpOnRust || await bind.mainGetProxyStatus());
    }

    final http.Response response;
    if (useFlutterHttp) {
      response =
          await _pollFlutterHttp(url, method, headers: headers, body: body);
    } else {
      String headersJson = jsonEncode(headers);
      String methodName = method.toString().split('.').last;
      final resJson = await bind
          .mainHttpRequest(
              url: url.toString(),
              method: methodName.toLowerCase(),
              body: body,
              header: headersJson)
          .single;
      response = _parseHttpResponse(resJson);
    }
    if (requestToken != null) {
      final [currentApi, currentToken] = await bind.mainAccountRequestContext();
      if (currentApi != requestApi || currentToken != requestToken) {
        throw http.ClientException('Account session changed', url);
      }
    }
    return response;
  }

  // Bounds only the pure-Dart branch below, which the OS would otherwise
  // let hang forever (e.g. a black-holed TLS handshake), see #15700.
  // The Rust branch has its own per-attempt timeouts.
  static const _requestTimeout = Duration(seconds: 30);

  Future<http.Response> _pollFlutterHttp(
    Uri url,
    HttpMethod method, {
    Map<String, String>? headers,
    dynamic body,
  }) async {
    final client = http.Client();
    try {
      var response = http.Response('', 400);

      switch (method) {
        case HttpMethod.get:
          response =
              await client.get(url, headers: headers).timeout(_requestTimeout);
          break;
        case HttpMethod.post:
          response = await client
              .post(url, headers: headers, body: body)
              .timeout(_requestTimeout);
          break;
        case HttpMethod.put:
          response = await client
              .put(url, headers: headers, body: body)
              .timeout(_requestTimeout);
          break;
        case HttpMethod.delete:
          response = await client
              .delete(url, headers: headers, body: body)
              .timeout(_requestTimeout);
          break;
        default:
          throw Exception('Unsupported HTTP method');
      }

      return response;
    } finally {
      client.close();
    }
  }

  http.Response _parseHttpResponse(String responseJson) {
    try {
      var parsedJson = jsonDecode(responseJson);
      final transportError = parsedJson['transport_error'];
      if (transportError is String) {
        throw http.ClientException(transportError);
      }
      String body = parsedJson['body'];
      Map<String, String> headers = {};
      for (var key in parsedJson['headers'].keys) {
        headers[key] = parsedJson['headers'][key];
      }
      int statusCode = parsedJson['status_code'];
      return http.Response(body, statusCode, headers: headers);
    } on http.ClientException {
      rethrow;
    } catch (_) {
      throw const FormatException('Failed to parse HTTP response');
    }
  }
}

Future<http.Response> get(Uri url, {Map<String, String>? headers}) async {
  return await HttpService().sendRequest(url, HttpMethod.get, headers: headers);
}

Future<http.Response> post(Uri url,
    {Map<String, String>? headers, Object? body, Encoding? encoding}) async {
  return await HttpService()
      .sendRequest(url, HttpMethod.post, body: body, headers: headers);
}

Future<http.Response> postAccountLogout({
  required String apiServer,
  required String token,
  required String id,
  required String uuid,
}) {
  return HttpService()._sendRequest(
    Uri.parse('$apiServer/api/logout'),
    HttpMethod.post,
    headers: {
      'Authorization': 'Bearer $token',
      'Content-Type': 'application/json',
    },
    body: jsonEncode({'id': id, 'uuid': uuid}),
    logoutContext: (apiServer, token),
  );
}

Future<http.Response> put(Uri url,
    {Map<String, String>? headers, Object? body, Encoding? encoding}) async {
  return await HttpService()
      .sendRequest(url, HttpMethod.put, body: body, headers: headers);
}

Future<http.Response> delete(Uri url,
    {Map<String, String>? headers, Object? body, Encoding? encoding}) async {
  return await HttpService()
      .sendRequest(url, HttpMethod.delete, body: body, headers: headers);
}
